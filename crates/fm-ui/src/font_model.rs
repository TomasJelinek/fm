//! Only visible font choices allocate Slint strings, even for large catalogs.
use super::*;

#[derive(Default)]
struct Cache {
    pointer: usize,
    length: usize,
    loaded: bool,
    family: String,
    missing: bool,
    index: i32,
    initialized: bool,
}

pub(super) struct FontRows {
    presenter: Rc<RefCell<Presenter>>,
    cache: RefCell<Cache>,
    notify: ModelNotify,
}

impl FontRows {
    pub(super) fn new(presenter: Rc<RefCell<Presenter>>) -> Self {
        Self {
            presenter,
            cache: RefCell::default(),
            notify: ModelNotify::default(),
        }
    }

    pub(super) fn refresh(&self, presenter: &Presenter) -> i32 {
        let fonts = &presenter.fonts;
        let family = &presenter.preferences.values.font_family;
        let changed = {
            let cache = self.cache.borrow();
            !cache.initialized
                || cache.pointer != fonts.catalog.as_ptr() as usize
                || cache.length != fonts.catalog.len()
                || cache.loaded != fonts.loaded
                || &cache.family != family
        };
        if changed {
            let index = presenter.font_index();
            *self.cache.borrow_mut() = Cache {
                pointer: fonts.catalog.as_ptr() as usize,
                length: fonts.catalog.len(),
                loaded: fonts.loaded,
                family: family.clone(),
                missing: !family.is_empty() && index as usize == fonts.catalog.len() + 1,
                index,
                initialized: true,
            };
            // Model reads can reenter; never notify while holding a mutable borrow.
            self.notify.reset();
        }
        self.cache.borrow().index
    }
}

impl Model for FontRows {
    type Data = SharedString;
    fn row_count(&self) -> usize {
        let cache = self.cache.borrow();
        1 + cache.length + usize::from(cache.missing)
    }
    fn row_data(&self, index: usize) -> Option<SharedString> {
        if index == 0 {
            return Some("Application default".into());
        }
        let cache = self.cache.borrow();
        if index <= cache.length {
            self.presenter
                .borrow()
                .fonts
                .catalog
                .get(index - 1)
                .map(|family| SharedString::from(family.as_str()))
        } else if cache.missing && index == cache.length + 1 {
            Some(cache.family.as_str().into())
        } else {
            None
        }
    }
    fn model_tracker(&self) -> &dyn ModelTracker {
        &self.notify
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
