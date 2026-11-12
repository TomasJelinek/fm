//! Find predicates reuse captured native facts, without panel folder exemptions.
use midnight_core::*;
use midnight_platform_common::{filter::CompiledFilter, filter_expression::CompiledExpression};

pub(super) enum Predicate {
    Name(CompiledFilter),
    Expression(CompiledExpression),
}
impl Predicate {
    pub(super) fn new(request: &FindRequest) -> Result<Self, String> {
        if request.name_mode == FilterMode::Expression {
            CompiledExpression::new(&request.masks, request.name_case_sensitive)
                .map(Self::Expression)
                .map_err(|error| {
                    format!(
                        "{} (bytes {}..{})",
                        error.message, error.span.start, error.span.end
                    )
                })
        } else {
            CompiledFilter::new(&PanelFilter {
                enabled: true,
                mode: request.name_mode,
                expression: request.masks.clone(),
                case_sensitive: request.name_case_sensitive,
            })
            .map(Self::Name)
        }
    }
    pub(super) fn matches(&self, entry: &FindEntry) -> bool {
        match self {
            Self::Name(filter) => entry
                .path
                .file_name()
                .is_some_and(|name| filter.matches(name)),
            Self::Expression(expression) => {
                let mut native = Entry::new(EntryId(0), entry.path.clone(), entry.kind);
                native.metadata = entry.metadata.clone();
                expression.matches(&native)
            }
        }
    }
}
