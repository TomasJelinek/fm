use super::*;
use crate::policy::Access;

fn bounded_text(value: Option<&CefString>, limit: usize) -> String {
    value
        .filter(|text| text.as_slice().is_none_or(|s| s.len() <= limit / 3))
        .map(ToString::to_string)
        .unwrap_or_default()
}

fn emit_layers(doc: &Document) {
    let Ok(surface) = doc.surface.lock() else {
        return;
    };
    let rgba = doc.layers.lock().ok().and_then(|layers| layers.frame());
    if let Some(rgba) = rgba {
        doc.emit(api::EventKind::Frame(api::Frame {
            viewport_revision: surface.revision,
            // CEF accepts inputs asynchronously; OnPaint does not supply their renderer
            // acknowledgment. Zero avoids claiming unproven pixel/input causality.
            acknowledged_input: 0,
            width: surface.width,
            height: surface.height,
            rgba,
        }));
    }
}

wrap_app! {
    pub(super) struct ViewerApp;
    impl App {
        fn on_before_command_line_processing(&self, _process_type: Option<&CefString>, command_line: Option<&mut CommandLine>) {
            if let Some(command_line) = command_line {
                for flag in ["disable-background-networking", "disable-component-update", "disable-extensions", "disable-sync", "no-first-run", "disable-default-apps", "disable-domain-reliability"] {
                    command_line.append_switch(Some(&CefString::from(flag)));
                }
            }
        }
    }
}

wrap_client! {
    pub(super) struct ViewerClient { doc: Arc<Document> }
    impl Client {
        fn render_handler(&self) -> Option<RenderHandler> { Some(Paint::new(self.doc.clone())) }
        fn request_handler(&self) -> Option<RequestHandler> { Some(Requests::new(self.doc.clone())) }
        fn life_span_handler(&self) -> Option<LifeSpanHandler> { Some(Life::new(self.doc.clone())) }
        fn find_handler(&self) -> Option<FindHandler> { Some(Search::new(self.doc.clone())) }
        fn display_handler(&self) -> Option<DisplayHandler> { Some(Display::new(self.doc.clone())) }
        fn load_handler(&self) -> Option<LoadHandler> { Some(Loading::new(self.doc.clone())) }
        fn keyboard_handler(&self) -> Option<KeyboardHandler> { Some(Keys::new(self.doc.clone())) }
        fn download_handler(&self) -> Option<DownloadHandler> { Some(NoDownloads::new()) }
        fn permission_handler(&self) -> Option<PermissionHandler> { Some(NoPermissions::new()) }
        fn dialog_handler(&self) -> Option<DialogHandler> { Some(NoFiles::new()) }
        fn jsdialog_handler(&self) -> Option<JsdialogHandler> { Some(NoDialogs::new()) }
        fn context_menu_handler(&self) -> Option<ContextMenuHandler> { Some(NoMenus::new()) }
    }
}

wrap_render_handler! {
    struct Paint { doc: Arc<Document> }
    impl RenderHandler {
        fn view_rect(&self, _browser: Option<&mut Browser>, rect: Option<&mut Rect>) {
            if let (Ok(viewport), Some(rect)) = (self.doc.viewport.lock(), rect) {
                *rect = Rect { x: 0, y: 0, width: (viewport.width as f64 / viewport.scale).ceil() as i32,
                    height: (viewport.height as f64 / viewport.scale).ceil() as i32 };
            }
        }
        fn screen_info(&self, _browser: Option<&mut Browser>, screen_info: Option<&mut ScreenInfo>) -> i32 {
            if let (Ok(viewport), Some(info)) = (self.doc.viewport.lock(), screen_info) {
                info.device_scale_factor = viewport.scale as f32;
                info.depth = 32;
                info.depth_per_component = 8;
                info.rect = Rect { x: 0, y: 0, width: viewport.width as i32, height: viewport.height as i32 };
                info.available_rect = info.rect.clone();
                return 1;
            }
            0
        }
        fn on_paint(&self, _browser: Option<&mut Browser>, type_: PaintElementType, _dirty_rects: Option<&[Rect]>, buffer: *const u8, width: i32, height: i32) {
            if buffer.is_null() || width <= 0 || height <= 0 || !self.doc.active.load(Ordering::Acquire) { return; }
            if type_ == PaintElementType::POPUP {
                if width > 8192 || height > 8192 || width as u64 * height as u64 > 1024 * 1024 { return; }
                // SAFETY: CEF callback guarantees this bounded width*height*4 BGRA allocation.
                let bgra = unsafe { std::slice::from_raw_parts(buffer, width as usize * height as usize * 4) };
                if let Ok(mut layers) = self.doc.layers.lock() { layers.popup(width as u32, height as u32, bgra); }
                emit_layers(&self.doc);
                return;
            }
            if type_ != PaintElementType::VIEW { return; }
            let Ok(mut surface) = self.doc.surface.lock() else { return; };
            // Check native dimensions before forming the borrowed slice or allocating pixels.
            let Ok(viewport) = self.doc.viewport.lock() else { return; };
            let native_width = ((viewport.width as f64 / viewport.scale).ceil() * viewport.scale).ceil() as u32;
            let native_height = ((viewport.height as f64 / viewport.scale).ceil() * viewport.scale).ceil() as u32;
            if width as u32 != native_width || height as u32 != native_height
                || width as u32 > surface.width + 16 || height as u32 > surface.height + 16 { return; }
            drop(viewport);
            let length = width as usize * height as usize * 4;
            // SAFETY: CEF guarantees a width*height*4 BGRA buffer for this OnPaint callback.
            // Surface dimensions were validated on Open/Resize, and the borrow stays here.
            let bgra = unsafe { std::slice::from_raw_parts(buffer, length) };
            let revision = surface.revision;
            if let Some(rgba) = surface.paint_cropped(revision, width as u32, height as u32, bgra) {
                if let Ok(mut layers) = self.doc.layers.lock() { layers.base(surface.width, surface.height, rgba); }
                drop(surface);
                emit_layers(&self.doc);
            }
        }
        fn on_popup_show(&self, _browser: Option<&mut Browser>, show: i32) {
            if let Ok(mut layers) = self.doc.layers.lock() { layers.show(show != 0); }
            emit_layers(&self.doc);
        }
        fn on_popup_size(&self, _browser: Option<&mut Browser>, rect: Option<&Rect>) {
            if let (Some(rect), Ok(viewport), Ok(mut layers)) = (rect, self.doc.viewport.lock(), self.doc.layers.lock()) {
                layers.popup_rect((rect.x as f64 * viewport.scale).round() as i32, (rect.y as f64 * viewport.scale).round() as i32);
            }
        }
        fn on_text_selection_changed(&self, _browser: Option<&mut Browser>, selected_text: Option<&CefString>, _selected_range: Option<&Range>) {
            if let Ok(mut selection) = self.doc.selection.lock() {
                selection.capture(selected_text.and_then(|text| text.as_slice()).unwrap_or_default());
            }
        }
    }
}

wrap_request_handler! {
    struct Requests { doc: Arc<Document> }
    impl RequestHandler {
        fn on_before_browse(&self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>, request: Option<&mut Request>, _user_gesture: i32, _is_redirect: i32) -> i32 {
            i32::from(!self.doc.active.load(Ordering::Acquire) || request.is_none_or(|r| self.doc.policy.access(&CefString::from(&r.url()).to_string()) == Access::Deny))
        }
        fn on_open_urlfrom_tab(&self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>, _target_url: Option<&CefString>, _target_disposition: WindowOpenDisposition, _user_gesture: i32) -> i32 { 1 }
        fn resource_request_handler(&self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>, request: Option<&mut Request>, _is_navigation: i32, is_download: i32, _request_initiator: Option<&CefString>, disable_default_handling: Option<&mut i32>) -> Option<ResourceRequestHandler> {
            if let Some(disable) = disable_default_handling {
                *disable = i32::from(is_download != 0 || request.is_none_or(|r| self.doc.policy.access(&CefString::from(&r.url()).to_string()) != Access::Network));
            }
            Some(resources::RequestPolicy::new(self.doc.clone()))
        }
        fn on_render_process_terminated(&self, _browser: Option<&mut Browser>, _status: TerminationStatus, _error_code: i32, _error_string: Option<&CefString>) {
            self.doc.error("Chromium renderer terminated; reopen the document");
        }
    }
}

wrap_life_span_handler! {
    struct Life { doc: Arc<Document> }
    impl LifeSpanHandler {
        fn on_before_popup(&self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>, _popup_id: i32, _target_url: Option<&CefString>, _target_frame_name: Option<&CefString>, _target_disposition: WindowOpenDisposition, _user_gesture: i32, _popup_features: Option<&PopupFeatures>, _window_info: Option<&mut WindowInfo>, _client: Option<&mut Option<Client>>, _settings: Option<&mut BrowserSettings>, _extra_info: Option<&mut Option<DictionaryValue>>, _no_javascript_access: Option<&mut i32>) -> i32 { 1 }
        fn on_before_close(&self, _browser: Option<&mut Browser>) { self.doc.closed.store(true, Ordering::Release); }
    }
}

wrap_find_handler! {
    struct Search { doc: Arc<Document> }
    impl FindHandler {
        fn on_find_result(&self, browser: Option<&mut Browser>, identifier: i32, count: i32, _selection_rect: Option<&Rect>, active_match_ordinal: i32, final_update: i32) {
            let sequence = self.doc.finding.lock().ok().and_then(|mut q| q.result(identifier, final_update != 0));
            let Some(sequence) = sequence else { return; };
            self.doc.emit(api::EventKind::FindResult(api::FindResult {
                request_sequence: sequence,
                matches: count.max(0) as u32, active: active_match_ordinal.max(0) as u32,
                complete: final_update != 0,
            }));
            if final_update != 0 && let Some(host) = browser.and_then(|b| b.host()) { input::dispatch_find(&self.doc, &host); }
        }
    }
}

wrap_load_handler! {
    struct Loading { doc: Arc<Document> }
    impl LoadHandler {
        fn on_loading_state_change(&self, browser: Option<&mut Browser>, is_loading: i32, _can_go_back: i32, _can_go_forward: i32) {
            if let Ok(mut finding) = self.doc.finding.lock() { finding.loaded(is_loading == 0); }
            let zoom = browser.as_ref().and_then(|b| b.host()).map(|h| 1.2f64.powf(h.zoom_level())).unwrap_or(1.0);
            self.doc.emit(api::EventKind::State(api::State { title: String::new(), url: self.doc.policy.main_url(), loading: is_loading != 0, editable: false, zoom }));
            if is_loading == 0 && let Some(host) = browser.and_then(|b| b.host()) { input::dispatch_find(&self.doc, &host); }
        }
    }
}

wrap_display_handler! {
    struct Display { doc: Arc<Document> }
    impl DisplayHandler {
        fn on_title_change(&self, browser: Option<&mut Browser>, title: Option<&CefString>) {
            self.doc.emit(api::EventKind::State(api::State { title: bounded_text(title, 4096), url: self.doc.policy.main_url(),
                loading: browser.as_ref().is_some_and(|b| b.is_loading() != 0), editable: false,
                zoom: browser.and_then(|b| b.host()).map(|h| 1.2f64.powf(h.zoom_level())).unwrap_or(1.0) }));
        }
        fn on_console_message(&self, _browser: Option<&mut Browser>, _level: LogSeverity, _message: Option<&CefString>, _source: Option<&CefString>, _line: i32) -> i32 { 1 }
    }
}

wrap_keyboard_handler! {
    struct Keys { doc: Arc<Document> }
    impl KeyboardHandler {
        fn on_pre_key_event(&self, _browser: Option<&mut Browser>, event: Option<&KeyEvent>, _os_event: Option<&mut crate::platform::NativeKeyEvent>, _is_keyboard_shortcut: Option<&mut i32>) -> i32 {
            let Some(event) = event else { return 0; };
            // Chromium supplies the current focused editable state for this key, rather than
            // relying on a delayed host/UI state flag. Suppress both raw key and subsequent CHAR.
            if event.windows_key_code == 32 && event.focus_on_editable_field == 0 && event.modifiers & (4 | 8 | 128) == 0 {
                if event.type_ == KeyEventType::RAWKEYDOWN || event.type_ == KeyEventType::KEYDOWN {
                    self.doc.emit(api::EventKind::HostShortcut(if event.modifiers & 2 != 0 { api::HostShortcut::CyclePrevious } else { api::HostShortcut::CycleNext }));
                }
                return 1;
            }
            0
        }
    }
}

wrap_download_handler! { struct NoDownloads; impl DownloadHandler {
    fn can_download(&self, _browser: Option<&mut Browser>, _url: Option<&CefString>, _request_method: Option<&CefString>) -> i32 { 0 }
    fn on_download_updated(&self, _browser: Option<&mut Browser>, _download_item: Option<&mut DownloadItem>, callback: Option<&mut DownloadItemCallback>) { if let Some(callback) = callback { callback.cancel(); } }
} }
wrap_permission_handler! { struct NoPermissions; impl PermissionHandler {
    fn on_request_media_access_permission(&self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>, _requesting_origin: Option<&CefString>, _requested_permissions: u32, callback: Option<&mut MediaAccessCallback>) -> i32 { if let Some(callback) = callback { callback.cont(0); } 1 }
    fn on_show_permission_prompt(&self, _browser: Option<&mut Browser>, _prompt_id: u64, _requesting_origin: Option<&CefString>, _requested_permissions: u32, callback: Option<&mut PermissionPromptCallback>) -> i32 { if let Some(callback) = callback { callback.cont(PermissionRequestResult::DENY); } 1 }
} }
wrap_dialog_handler! { struct NoFiles; impl DialogHandler {
    fn on_file_dialog(&self, _browser: Option<&mut Browser>, _mode: FileDialogMode, _title: Option<&CefString>, _default_file_path: Option<&CefString>, _accept_filters: Option<&mut CefStringList>, _accept_extensions: Option<&mut CefStringList>, _accept_descriptions: Option<&mut CefStringList>, callback: Option<&mut FileDialogCallback>) -> i32 { if let Some(callback) = callback { callback.cancel(); } 1 }
} }
wrap_jsdialog_handler! { struct NoDialogs; impl JsdialogHandler {
    fn on_jsdialog(&self, _browser: Option<&mut Browser>, _origin_url: Option<&CefString>, _dialog_type: JsdialogType, _message_text: Option<&CefString>, _default_prompt_text: Option<&CefString>, _callback: Option<&mut JsdialogCallback>, suppress_message: Option<&mut i32>) -> i32 { if let Some(suppress) = suppress_message { *suppress = 1; } 0 }
    fn on_before_unload_dialog(&self, _browser: Option<&mut Browser>, _message_text: Option<&CefString>, _is_reload: i32, callback: Option<&mut JsdialogCallback>) -> i32 { if let Some(callback) = callback { callback.cont(1, None); } 1 }
} }
wrap_context_menu_handler! { struct NoMenus; impl ContextMenuHandler {
    fn on_before_context_menu(&self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>, _params: Option<&mut ContextMenuParams>, model: Option<&mut MenuModel>) { if let Some(model) = model { model.clear(); } }
} }
