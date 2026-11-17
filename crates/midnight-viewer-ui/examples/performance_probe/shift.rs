//! Shared sub-pixel shift fit for the diagnostic pixel comparisons.

/// `errors[i]` is the squared error for a shift of `i - maximum` pixels. Returns
/// the best shift refined by a parabola through its neighbors, and the best error
/// relative to no movement (1.0 when nothing differed).
pub(super) fn fit(errors: &[f64], maximum: usize) -> Option<(f64, f64)> {
    let (index, best) = errors
        .iter()
        .enumerate()
        .min_by(|a, b| a.1.total_cmp(b.1))?;
    let mut shift = index as f64 - maximum as f64;
    if index > 0 && index + 1 < errors.len() {
        let left = errors[index - 1];
        let right = errors[index + 1];
        let curvature = left - 2.0 * best + right;
        if curvature > 0.0 {
            shift += ((left - right) / (2.0 * curvature)).clamp(-0.5, 0.5);
        }
    }
    let unmoved = *errors.get(maximum)?;
    Some((shift, if unmoved == 0.0 { 1.0 } else { best / unmoved }))
}
