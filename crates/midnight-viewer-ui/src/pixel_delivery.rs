//! Deliver already prepared pixels; native image creation only retains their buffers.
use super::*;

pub(super) fn bind(
    open: &Rc<OpenWindow>,
    service: Service,
    copy: Rc<dyn Fn(String)>,
) -> Result<(), String> {
    let weak = Rc::downgrade(open);
    open.window.on_deliver_output(move || {
        if let Some(open) = weak.upgrade()
            && !open.closed.get()
        {
            receive(&open, &service, copy.as_ref());
        }
    });
    let window = open.window.as_weak();
    let preparation = pixel_preparation::Preparation::new(
        open.session.mailbox.clone(),
        std::sync::Arc::new(move || {
            window
                .upgrade_in_event_loop(|window| window.invoke_deliver_output())
                .is_ok()
        }),
    )?;
    *open.preparation.borrow_mut() = Some(preparation);
    Ok(())
}

pub(super) fn receive(open: &OpenWindow, service: &Service, copy: &dyn Fn(String)) {
    let Some(batch) = open
        .preparation
        .borrow()
        .as_ref()
        .and_then(pixel_preparation::Preparation::take)
    else {
        return;
    };
    if open.closed.get() {
        service.retire(batch);
        return;
    }
    let mut outputs = batch.outputs;
    let revision = outputs.iter().find_map(|output| match output {
        ViewerOutput::Frame(frame) => Some(frame.revision),
        _ => None,
    });
    let mut previous = None;
    match batch.pixels {
        Ok(pixels) if revision.is_some() => {
            previous = Some(open.prepared_pixels.replace(pixels));
        }
        Ok(_) => {}
        Err(error) => {
            let message = format!("Cannot prepare viewer pixels: {error}");
            open.opened(Err(message.clone()));
            outputs = outputs
                .into_iter()
                .filter_map(|output| {
                    if matches!(output, ViewerOutput::Frame(_)) {
                        service.retire(output);
                        None
                    } else {
                        Some(output)
                    }
                })
                .collect();
            outputs.push(ViewerOutput::Status(message));
        }
    }
    present_outputs(open, service, copy, outputs);
    if let Some(previous) = previous {
        if open.closed.get() || revision == Some(open.frame.get()) {
            // Hold the last displayed buffer through the native image replacement,
            // then release it away from the event loop. This adds no pixel copy.
            service.retire(previous);
        } else {
            // A stale or rejected frame cannot replace the accepted buffer lease.
            service.retire(open.prepared_pixels.replace(previous));
        }
    }
}

pub(super) fn pixels(
    open: &OpenWindow,
    frame: &ViewerFrame,
) -> Result<pixel_preparation::PreparedPixels, String> {
    if let Some(pixels) = open.prepared_pixels.borrow().as_ref()
        && pixels.frame_revision == frame.revision
    {
        return Ok(pixels.clone());
    }
    // Deterministic UI regressions inject frames directly without a running
    // mailbox. Production never performs the large pixel copy on this thread.
    #[cfg(test)]
    {
        return pixel_preparation::prepare(frame)?
            .ok_or_else(|| "Frame has no raster pixels.".into());
    }
    #[cfg(not(test))]
    Err("Prepared viewer pixels do not match the current frame.".into())
}
