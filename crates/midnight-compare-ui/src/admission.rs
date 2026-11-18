//! One metadata refresh is permitted only before a comparison Session exists.
use midnight_compare_engine::{OpenError, Service, Session, Wake};
use midnight_core::{FileCompareLaunch, ViewerAppearance, ViewerGeometry};

#[cfg(test)]
pub(super) fn open(
    service: &Service,
    launch: FileCompareLaunch,
    appearance: ViewerAppearance,
    geometry: ViewerGeometry,
) -> Result<Session, OpenError> {
    open_impl(service, launch, appearance, geometry, None, |_| {})
}

pub(super) fn open_with_wake(
    service: &Service,
    launch: FileCompareLaunch,
    appearance: ViewerAppearance,
    geometry: ViewerGeometry,
    wake: Wake,
) -> Result<Session, OpenError> {
    open_impl(service, launch, appearance, geometry, Some(wake), |_| {})
}

#[cfg(test)]
pub(super) fn open_observed(
    service: &Service,
    launch: FileCompareLaunch,
    appearance: ViewerAppearance,
    geometry: ViewerGeometry,
    observe: impl FnMut(&FileCompareLaunch),
) -> Result<Session, OpenError> {
    open_impl(service, launch, appearance, geometry, None, observe)
}

fn open_impl(
    service: &Service,
    mut launch: FileCompareLaunch,
    appearance: ViewerAppearance,
    geometry: ViewerGeometry,
    wake: Option<Wake>,
    mut observe: impl FnMut(&FileCompareLaunch),
) -> Result<Session, OpenError> {
    let admit = |launch, appearance| match &wake {
        Some(wake) => service.open_with_wake(launch, appearance, geometry, wake.clone()),
        None => service.open(launch, appearance, geometry),
    };
    observe(&launch);
    match admit(launch.clone(), appearance.clone()) {
        Err(OpenError::PolicyChanged) => {
            let current = service.comparator_snapshot();
            launch.comparators = current.preferences;
            launch.comparator_policy = current.id;
            observe(&launch);
            admit(launch, appearance)
        }
        result => result,
    }
}
