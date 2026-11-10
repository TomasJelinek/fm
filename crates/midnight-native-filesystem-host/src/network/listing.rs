use super::*;
use std::collections::HashSet;

impl NetworkProvider {
    pub(super) fn list_inner(
        &self,
        address: &NetworkAddress,
        cancelled: Cancel<'_>,
        emit: &mut dyn FnMut(NetworkNode) -> bool,
    ) -> Result<(), Failure> {
        self.client.require(abi::CAP_BROWSE)?;
        let (_mount, mount, directory) = self.resolve(address, cancelled)?;
        let mut response = self.client.exchange(
            Request::ListBegin {
                mount,
                directory: directory.id,
                // Publish the first device without waiting for a slow second provider result.
                max_entries: 1,
            },
            cancelled,
        )?;
        let cursor = match &response {
            Response::Page { cursor, .. } if *cursor != 0 => *cursor,
            _ => return Err(invalid("Invalid network listing cursor")),
        };
        let _cursor = Lease::new(&self.client, Request::CloseList { cursor });
        let mut expected_page = 0;
        let mut seen = HashSet::new();
        let mut addresses = HashSet::new();
        let mut bytes = 0usize;
        loop {
            let Response::Page {
                cursor: received,
                page,
                done,
                entries,
            } = response
            else {
                return Err(invalid("Missing network listing page"));
            };
            if received != cursor && received != 0 {
                drop(Lease::new(
                    &self.client,
                    Request::CloseList { cursor: received },
                ));
            }
            if received != cursor
                || page != expected_page
                || entries.len() > 128
                || (!done && entries.is_empty())
            {
                return Err(invalid("Invalid network page sequence"));
            }
            check_cancel(cancelled)?;
            for node in entries {
                check_cancel(cancelled)?;
                validate_node(&node)?;
                bytes = bytes
                    .checked_add(node.address.bytes.len() + node.display.len())
                    .ok_or_else(|| invalid("Network listing exceeds memory budget"))?;
                if seen.len() >= 4096
                    || bytes > 16 * 1024 * 1024
                    || !seen.insert(node.id)
                    || !addresses.insert(node.address.bytes.clone())
                {
                    return Err(invalid("Network listing exceeds budget or repeats a node"));
                }
                let entry = NetworkNode {
                    address: self.decode(node.address, node.display.clone())?,
                    display: node.display,
                };
                if !emit(entry) {
                    return Ok(());
                }
            }
            if done {
                return Ok(());
            }
            expected_page += 1;
            response = self.client.exchange(
                Request::ListNext {
                    cursor,
                    expected_page,
                    max_entries: 128,
                },
                cancelled,
            )?;
        }
    }
}
