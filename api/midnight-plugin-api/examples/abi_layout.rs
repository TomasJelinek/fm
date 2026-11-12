use midnight_plugin_api::abi::*;
use std::mem::{align_of, offset_of, size_of};

macro_rules! layout {
    ($type:ty, $($field:ident),+) => {
        print!("{} {} {}", stringify!($type), size_of::<$type>(), align_of::<$type>());
        $(print!(" {}", offset_of!($type, $field));)+
        println!();
    };
}

fn main() {
    layout!(
        Header,
        struct_size,
        abi_major,
        abi_minor,
        kind,
        minimum_host_minor,
        reserved
    );
    layout!(ByteSlice, data, len, reserved);
    layout!(Buffer, bytes, id);
    layout!(
        Request, id, session, generation, kind, flags, payload, content
    );
    layout!(
        Event, request_id, session, generation, kind, flags, payload, content
    );
    layout!(Work, context, run, drop, request_id, class, affinity);
    layout!(
        HostApi,
        header,
        context,
        schedule_work,
        cancelled,
        submit_service,
        notify_ready,
        retain_buffer,
        release_buffer,
        diagnostic
    );
    layout!(
        PluginApi,
        header,
        id,
        version,
        create,
        submit,
        cancel,
        drain,
        release_buffer,
        begin_shutdown,
        destroy
    );
}
