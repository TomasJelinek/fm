use crate::viewer::{Session, fail};
use midnight_plugin_sdk::{
    Context,
    api::{abi, messages},
};
use std::sync::Arc;

type Done = Box<dyn FnOnce(Context, Vec<u8>, bool) + Send>;
// A short successful read is not EOF. Accumulate only this bounded request;
// copied bytes outlive individual SDK content leases, each released at callback end.
pub fn read(context: Context, session: Arc<Session>, offset: u64, count: usize, done: Done) {
    let count = count.min(crate::decode::READ_BYTES);
    let count = session
        .open
        .length
        .map_or(count, |len| count.min(len.saturating_sub(offset) as usize));
    let count = count.min(u64::MAX.saturating_sub(offset) as usize);
    gather(
        context,
        session,
        offset,
        count,
        Vec::with_capacity(count),
        0,
        done,
    );
}
fn gather(
    context: Context,
    session: Arc<Session>,
    offset: u64,
    count: usize,
    mut bytes: Vec<u8>,
    stalls: u8,
    done: Done,
) {
    if !session.live(&context) {
        return;
    }
    if bytes.len() == count {
        let eof = session
            .open
            .length
            .is_some_and(|len| offset + bytes.len() as u64 >= len);
        done(context, bytes, eof);
        return;
    }
    let request = messages::ReadAt {
        reader: session.open.reader,
        revision: session.open.revision,
        offset: offset + bytes.len() as u64,
        length: (count - bytes.len()) as u32,
    };
    let result = context.read(
        request,
        Box::new(move |context, reply, content| {
            if !session.live(&context) {
                return;
            }
            if reply.status != abi::OK {
                fail(&context, reply.status, "Content read failed");
                return;
            }
            let empty = content.as_ref().is_empty();
            bytes.extend_from_slice(content.as_ref());
            drop(content);
            if reply.eof {
                done(context, bytes, true);
                return;
            }
            let stalls = if empty { stalls + 1 } else { 0 };
            if stalls >= 3 {
                fail(
                    &context,
                    abi::INVALID,
                    "Reader repeatedly returned no data without EOF",
                );
                return;
            }
            gather(context, session, offset, count, bytes, stalls, done);
        }),
    );
    if let Err(status) = result {
        fail(&context, status, "Unable to schedule content read");
    }
}
