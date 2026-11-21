//! The chain of a shown location's ancestors, as its provider reports them, for the
//! path bar. Parent lookups resolve provider addresses without discovery.
use super::*;

/// The most ancestors published for one location. The bundled provider nests at most
/// 64 levels, so its chains are never cut.
pub(super) const MAX_ANCESTORS: usize = 64;
/// The most address and caption bytes of the ancestors beyond the parent.
pub(super) const MAX_ANCESTOR_BYTES: usize = 16 * 1024 * 1024;

/// The ancestors of `address`, the outermost first: its provider's parent, then that
/// one's parent, up to a root. Up opens the parent, so a failure to report it fails the
/// request, as before. Further out, a failed lookup, a location already in the chain or
/// the bounds end the chain with the nearer ancestors already known, and the path bar
/// then starts below the root. Nothing is inferred from how the location was reached,
/// so the chain is the same after Enter, Up, Back, a restored tab or a Hot Path.
pub(super) fn ancestors(
    provider: &dyn NetworkNeighborhood,
    address: &NetworkAddress,
    cancel: &Cancellation,
) -> Result<Arc<[NetworkAddress]>, NetworkFailure> {
    let mut chain: Vec<NetworkAddress> = Vec::new();
    let mut bytes = 0_usize;
    let mut next = provider.parent(address, cancel)?;
    while let Some(ancestor) = next.take() {
        check_cancel(cancel)?;
        // The parent itself is kept as reported; only those beyond it are bounded.
        if !chain.is_empty() {
            bytes = bytes
                .saturating_add(ancestor.opaque.len())
                .saturating_add(ancestor.display.len());
            let repeated = same(&ancestor, address) || chain.iter().any(|a| same(a, &ancestor));
            if repeated || chain.len() >= MAX_ANCESTORS || bytes > MAX_ANCESTOR_BYTES {
                break;
            }
        }
        next = match provider.parent(&ancestor, cancel) {
            Ok(next) => next,
            Err(_) => {
                check_cancel(cancel)?;
                None
            }
        };
        chain.push(ancestor);
    }
    chain.reverse();
    Ok(chain.into())
}

/// Identity is the provider and its opaque bytes; a caption is presentation only.
fn same(a: &NetworkAddress, b: &NetworkAddress) -> bool {
    a.provider == b.provider && a.opaque == b.opaque
}

#[cfg(test)]
#[path = "ancestor_tests.rs"]
mod tests;
