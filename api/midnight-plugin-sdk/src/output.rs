use crate::outbox::Output;
use midnight_plugin_api::{abi, codec, document::DocumentPage, messages, viewer, wire::WireError};

impl Output {
    pub fn opened(value: &messages::Opened) -> Result<Self, WireError> {
        Self::encode(abi::EVENT_OPENED, value)
    }
    pub fn text(value: &viewer::TextPage) -> Result<Self, WireError> {
        Self::encode(abi::EVENT_TEXT_PAGE, value)
    }
    pub fn document(value: &DocumentPage) -> Result<Self, WireError> {
        Self::encode(abi::EVENT_DOCUMENT_PAGE, value)
    }
    pub fn search_match(value: &messages::SearchMatch) -> Result<Self, WireError> {
        Self::encode(abi::EVENT_SEARCH_MATCH, value)
    }
    pub fn progress(value: &messages::Progress) -> Result<Self, WireError> {
        Self::encode(abi::EVENT_PROGRESS, value)
    }
    pub fn text_position(value: &viewer::TextPosition) -> Result<Self, WireError> {
        Self::encode(abi::EVENT_TEXT_POSITION, value)
    }

    pub fn binary(value: &viewer::BinaryPage, content: Box<[u8]>) -> Result<Self, WireError> {
        if value.source.length != content.len() as u64 {
            return Err(WireError::Malformed);
        }
        let mut output = Self::encode(abi::EVENT_BINARY_PAGE, value)?;
        output.content = Some(content);
        Ok(output)
    }

    fn encode<T: codec::Model>(kind: u32, value: &T) -> Result<Self, WireError> {
        Ok(Self {
            kind,
            payload: codec::encode(value)?.into_boxed_slice(),
            content: None,
        })
    }

    pub(crate) fn byte_len(&self) -> usize {
        self.payload.len() + self.content.as_ref().map_or(0, |bytes| bytes.len())
    }
}

#[cfg(test)]
mod tests {
    use crate::outbox::Output;
    use midnight_plugin_api::{
        viewer::{BinaryPage, SourceRange},
        wire::WireError,
    };

    #[test]
    fn binary_output_requires_exact_raw_length_and_bounded_source_range() {
        let page = BinaryPage {
            revision: 1,
            source: SourceRange {
                offset: 1u64 << 35,
                length: 3,
            },
            has_before: true,
            has_after: false,
        };
        assert!(Output::binary(&page, vec![0, 128, 255].into_boxed_slice()).is_ok());
        assert_eq!(
            Output::binary(&page, Box::new([])).unwrap_err(),
            WireError::Malformed
        );
    }
}
