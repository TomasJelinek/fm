//! Network discovery shares the audited C transport, never the native path API.
use crate::{
    Cancel, NativeProvider, Package,
    client::{Lease, check_cancel},
    discovery::invalid,
};
use midnight_core::{NetworkAddress, NetworkDestination, NetworkNode};
use midnight_filesystem_plugin_api::{
    ActivationTarget, Address, AddressEncoding, Failure, Kind, Node, Request, Response, abi,
};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct NetworkProvider {
    id: String,
    client: NativeProvider,
}

impl NetworkProvider {
    fn encode(&self, address: &NetworkAddress) -> Result<Address, Failure> {
        if address.provider != self.id {
            return Err(invalid("Foreign network provider address"));
        }
        // Admit the caller-owned identity before copying its opaque allocation.
        address.validate().map_err(invalid)?;
        let address = Address {
            encoding: AddressEncoding::Opaque,
            bytes: address.opaque.to_vec(),
        };
        validate_address(&address)?;
        Ok(address)
    }
    fn decode(&self, address: Address, display: String) -> Result<NetworkAddress, Failure> {
        validate_address(&address)?;
        if display.len() > 16 * 1024 || display.contains('\0') {
            return Err(invalid("Invalid network display name"));
        }
        Ok(NetworkAddress {
            provider: self.id.clone(),
            opaque: address.bytes.into(),
            display,
        })
    }
    fn resolved(
        &self,
        request: Request,
        cancelled: Cancel<'_>,
    ) -> Result<(Lease, u64, Node), Failure> {
        let Response::Resolved { mount, node } = self.client.exchange(request, cancelled)? else {
            return Err(invalid("Missing resolved network node"));
        };
        let lease = Lease::new(&self.client, Request::CloseMount { mount });
        check_cancel(cancelled)?;
        if mount == 0 {
            return Err(invalid("Invalid network mount"));
        }
        validate_node(&node)?;
        Ok((lease, mount, node))
    }
    fn resolve(
        &self,
        address: &NetworkAddress,
        cancelled: Cancel<'_>,
    ) -> Result<(Lease, u64, Node), Failure> {
        self.resolved(
            Request::Resolve {
                address: self.encode(address)?,
            },
            cancelled,
        )
    }
    fn activate_inner(
        &self,
        address: &NetworkAddress,
        cancelled: Cancel<'_>,
    ) -> Result<NetworkDestination, Failure> {
        self.client.require(abi::CAP_ACTIVATE)?;
        let (_lease, mount, node) = self.resolve(address, cancelled)?;
        let Response::Activated(target) = self.client.exchange(
            Request::Activate {
                mount,
                node: node.id,
            },
            cancelled,
        )?
        else {
            return Err(invalid("Missing activation target"));
        };
        check_cancel(cancelled)?;
        target.validate().map_err(invalid)?;
        match target {
            ActivationTarget::WithinProvider(next) => {
                Ok(NetworkDestination::Browse(self.decode(next, node.display)?))
            }
            ActivationTarget::Native(path) => {
                Ok(NetworkDestination::Native(crate::paths::decode(&path)?))
            }
        }
    }
    pub fn open(package: &Package) -> Result<Self, Failure> {
        if package.encoding != AddressEncoding::Opaque {
            return Err(invalid(
                "Network discovery requires an opaque-address provider",
            ));
        }
        Ok(Self {
            id: package.id.clone(),
            client: NativeProvider {
                transport: Arc::new(crate::ffi::Instance::open(package)?),
                capabilities: package.capabilities,
            },
        })
    }
}

fn validate_address(address: &Address) -> Result<(), Failure> {
    if address.encoding != AddressEncoding::Opaque
        || address.bytes.is_empty()
        || address.bytes.len() > 16 * 1024 * 1024
    {
        return Err(invalid("Invalid opaque network address"));
    }
    Ok(())
}
fn validate_node(node: &Node) -> Result<(), Failure> {
    validate_address(&node.address)?;
    if node.id == 0
        || node.kind != Kind::Directory
        || node.display.contains('\0')
        || node.display.len() > 16 * 1024
    {
        return Err(invalid("Invalid network discovery node"));
    }
    Ok(())
}

mod listing;
mod service;

#[cfg(all(test, target_os = "linux"))]
mod installed_linux_tests;
#[cfg(test)]
mod installed_tests;
#[cfg(test)]
mod tests;
