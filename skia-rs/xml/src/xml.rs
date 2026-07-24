use std::{
    collections::{HashMap, HashSet},
    fmt,
    sync::Arc,
};

const XML_NAMESPACE: &str = "http://www.w3.org/XML/1998/namespace";
const XMLNS_NAMESPACE: &str = "http://www.w3.org/2000/xmlns/";

/// Stable machine-readable XML parsing failure code.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum XmlErrorCode {
    /// One configured parser ceiling is zero or otherwise invalid.
    InvalidLimits,
    /// The input byte sequence is not UTF-8.
    InvalidUtf8,
    /// XML syntax is malformed.
    Malformed,
    /// The document has no root element or has non-whitespace trailing content.
    InvalidDocument,
    /// An element or attribute name is outside the supported XML name subset.
    InvalidName,
    /// A qualified name or namespace declaration is invalid or unresolved.
    InvalidNamespace,
    /// One element declared the same attribute more than once.
    DuplicateAttribute,
    /// A DTD, declaration, or arbitrary entity was requested.
    UnsupportedEntity,
    /// A document type declaration was requested.
    UnsupportedDoctype,
    /// The XML declaration requests a version other than XML 1.0.
    UnsupportedVersion,
    /// The XML declaration requests an encoding other than UTF-8.
    UnsupportedEncoding,
    /// An XML character is forbidden by the XML character range.
    InvalidCharacter,
    /// An input, nesting, node, attribute, name, or text ceiling was exceeded.
    ResourceLimit,
    /// Memory needed for bounded parsing could not be allocated.
    AllocationFailed,
}

/// Source-redacted XML parse error with a byte offset into the original input.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct XmlError {
    code: XmlErrorCode,
    offset: usize,
}

impl XmlError {
    const fn new(code: XmlErrorCode, offset: usize) -> Self {
        Self { code, offset }
    }

    /// Returns the stable failure category.
    pub const fn code(self) -> XmlErrorCode {
        self.code
    }

    /// Returns the zero-based byte offset at which parsing failed.
    pub const fn offset(self) -> usize {
        self.offset
    }
}

impl fmt::Display for XmlError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:?} at byte {}", self.code, self.offset)
    }
}

impl std::error::Error for XmlError {}

/// Hard ceilings applied before parsing one untrusted XML document.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct XmlLimits {
    /// Maximum accepted input byte count.
    pub max_input_bytes: usize,
    /// Maximum open element depth, including the root element.
    pub max_depth: usize,
    /// Maximum retained element and text node count.
    pub max_nodes: usize,
    /// Maximum attributes on one element.
    pub max_attributes_per_element: usize,
    /// Maximum byte count of one element or attribute name.
    pub max_name_bytes: usize,
    /// Maximum decoded byte count of one text node.
    pub max_text_bytes: usize,
    /// Maximum decoded byte count of one attribute value.
    pub max_attribute_value_bytes: usize,
    /// Maximum decoded byte count across all retained text nodes.
    pub max_total_text_bytes: usize,
}

impl XmlLimits {
    /// Validates that every configured ceiling is positive.
    pub fn validate(self) -> Result<Self, XmlError> {
        if self.max_input_bytes == 0
            || self.max_depth == 0
            || self.max_nodes == 0
            || self.max_attributes_per_element == 0
            || self.max_name_bytes == 0
            || self.max_text_bytes == 0
            || self.max_attribute_value_bytes == 0
            || self.max_total_text_bytes == 0
        {
            return Err(XmlError::new(XmlErrorCode::InvalidLimits, 0));
        }
        Ok(self)
    }
}

impl Default for XmlLimits {
    fn default() -> Self {
        Self {
            max_input_bytes: 16 * 1024 * 1024,
            max_depth: 128,
            max_nodes: 1_000_000,
            max_attributes_per_element: 256,
            max_name_bytes: 512,
            max_text_bytes: 4 * 1024 * 1024,
            max_attribute_value_bytes: 256 * 1024,
            max_total_text_bytes: 16 * 1024 * 1024,
        }
    }
}

/// One parsed XML document with exactly one root element.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct XmlDocument {
    root: XmlElement,
}

impl XmlDocument {
    /// Parses one bounded UTF-8 XML document.
    ///
    /// The parser supports elements, attributes, text, CDATA sections,
    /// comments, processing instructions, the five predefined XML entities,
    /// and numeric character references. It rejects DTDs, custom entities,
    /// external entities, and declarations that could introduce them.
    pub fn parse(input: &[u8], limits: XmlLimits) -> Result<Self, XmlError> {
        let limits = limits.validate()?;
        if input.len() > limits.max_input_bytes {
            return Err(XmlError::new(
                XmlErrorCode::ResourceLimit,
                limits.max_input_bytes,
            ));
        }
        let input = std::str::from_utf8(input)
            .map_err(|error| XmlError::new(XmlErrorCode::InvalidUtf8, error.valid_up_to()))?;
        let mut document = Parser::new(input, limits).parse_document()?;
        resolve_namespaces(&mut document.root)?;
        Ok(document)
    }

    /// Borrows the document's one root element.
    pub const fn root(&self) -> &XmlElement {
        &self.root
    }

    /// Consumes the document and returns its root element.
    pub fn into_root(self) -> XmlElement {
        self.root
    }
}

/// One XML element with owned names, attributes, and child nodes.
#[derive(Clone, Debug)]
pub struct XmlElement {
    name: String,
    name_offset: usize,
    namespace_uri: Option<Arc<str>>,
    attributes: Vec<XmlAttribute>,
    children: Vec<XmlNode>,
}

impl PartialEq for XmlElement {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
            && self.namespace_uri == other.namespace_uri
            && self.attributes == other.attributes
            && self.children == other.children
    }
}

impl Eq for XmlElement {}

impl XmlElement {
    /// Returns the qualified element name as it appeared in the source.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the optional lexical prefix.
    pub fn prefix(&self) -> Option<&str> {
        split_qualified_name(&self.name).0
    }

    /// Returns the local part of the element name.
    pub fn local_name(&self) -> &str {
        split_qualified_name(&self.name).1
    }

    /// Returns the resolved namespace URI.
    pub fn namespace_uri(&self) -> Option<&str> {
        self.namespace_uri.as_deref()
    }

    /// Returns the qualified name's byte offset in the original UTF-8 input.
    pub const fn name_offset(&self) -> usize {
        self.name_offset
    }

    /// Borrows attributes in declaration order.
    pub fn attributes(&self) -> &[XmlAttribute] {
        &self.attributes
    }

    /// Finds one attribute by its exact qualified name.
    pub fn attribute(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|attribute| attribute.name == name)
            .map(|attribute| attribute.value.as_str())
    }

    /// Finds an attribute by expanded namespace URI and local name.
    pub fn attribute_ns(&self, namespace_uri: Option<&str>, local_name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|attribute| {
                attribute.namespace_uri() == namespace_uri && attribute.local_name() == local_name
            })
            .map(|attribute| attribute.value.as_str())
    }

    /// Borrows child nodes in declaration order.
    pub fn children(&self) -> &[XmlNode] {
        &self.children
    }
}

/// One XML attribute with an entity-decoded value.
#[derive(Clone, Debug)]
pub struct XmlAttribute {
    name: String,
    name_offset: usize,
    namespace_uri: Option<Arc<str>>,
    value: String,
}

impl PartialEq for XmlAttribute {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
            && self.namespace_uri == other.namespace_uri
            && self.value == other.value
    }
}

impl Eq for XmlAttribute {}

impl XmlAttribute {
    /// Returns the qualified attribute name as it appeared in the source.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the optional lexical prefix.
    pub fn prefix(&self) -> Option<&str> {
        split_qualified_name(&self.name).0
    }

    /// Returns the local part of the attribute name.
    pub fn local_name(&self) -> &str {
        split_qualified_name(&self.name).1
    }

    /// Returns the resolved namespace URI.
    pub fn namespace_uri(&self) -> Option<&str> {
        self.namespace_uri.as_deref()
    }

    /// Returns the qualified name's byte offset in the original UTF-8 input.
    pub const fn name_offset(&self) -> usize {
        self.name_offset
    }

    /// Returns the entity-decoded attribute value.
    pub fn value(&self) -> &str {
        &self.value
    }
}

/// One retained XML child node.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum XmlNode {
    /// One nested element.
    Element(XmlElement),
    /// Entity-decoded text or CDATA content.
    Text(String),
}

impl XmlNode {
    /// Returns the nested element when this is an element node.
    pub const fn as_element(&self) -> Option<&XmlElement> {
        match self {
            Self::Element(element) => Some(element),
            Self::Text(_) => None,
        }
    }

    /// Returns the decoded text when this is a text node.
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Element(_) => None,
            Self::Text(text) => Some(text),
        }
    }
}

struct Parser<'a> {
    input: &'a str,
    limits: XmlLimits,
    offset: usize,
    node_count: usize,
    total_text_bytes: usize,
}

impl<'a> Parser<'a> {
    fn new(input: &'a str, limits: XmlLimits) -> Self {
        let offset = usize::from(input.starts_with('\u{FEFF}')) * '\u{FEFF}'.len_utf8();
        Self {
            input,
            limits,
            offset,
            node_count: 0,
            total_text_bytes: 0,
        }
    }

    fn parse_document(mut self) -> Result<XmlDocument, XmlError> {
        if self.starts_with("<?xml")
            && matches!(
                self.peek_at("<?xml".len()),
                Some(b' ' | b'\t' | b'\r' | b'\n')
            )
        {
            self.parse_xml_declaration()?;
        }
        self.skip_document_misc()?;
        if self.peek() != Some(b'<') {
            return Err(self.error(XmlErrorCode::InvalidDocument));
        }
        let root = self.parse_element(1)?;
        self.skip_document_misc()?;
        if self.offset != self.input.len() {
            return Err(self.error(XmlErrorCode::InvalidDocument));
        }
        Ok(XmlDocument { root })
    }

    fn parse_xml_declaration(&mut self) -> Result<(), XmlError> {
        let declaration_start = self.offset;
        self.offset += "<?xml".len();
        let content_start = self.offset;
        let end = self.find("?>").ok_or(self.error(XmlErrorCode::Malformed))?;
        validate_xml_declaration(
            &self.input[content_start..end],
            content_start,
            declaration_start,
        )?;
        self.offset = end + "?>".len();
        Ok(())
    }

    fn skip_document_misc(&mut self) -> Result<(), XmlError> {
        loop {
            self.skip_whitespace();
            if self.starts_with("<!--") {
                self.skip_comment()?;
            } else if self.starts_with("<?") {
                self.skip_processing_instruction()?;
            } else if self.starts_with("<!DOCTYPE") {
                return Err(self.error(XmlErrorCode::UnsupportedDoctype));
            } else {
                return Ok(());
            }
        }
    }

    fn parse_element(&mut self, depth: usize) -> Result<XmlElement, XmlError> {
        if depth > self.limits.max_depth {
            return Err(self.error(XmlErrorCode::ResourceLimit));
        }
        self.expect_byte(b'<')?;
        if self.peek() == Some(b'/') || self.peek() == Some(b'!') || self.peek() == Some(b'?') {
            return Err(self.error(XmlErrorCode::Malformed));
        }
        self.reserve_node()?;
        let name_offset = self.offset;
        let name = self.parse_name()?;
        let mut attributes = Vec::new();
        loop {
            self.skip_whitespace();
            match self.peek() {
                Some(b'>') => {
                    self.offset += 1;
                    break;
                }
                Some(b'/') if self.peek_at(1) == Some(b'>') => {
                    self.offset += 2;
                    return Ok(XmlElement {
                        name,
                        name_offset,
                        namespace_uri: None,
                        attributes,
                        children: Vec::new(),
                    });
                }
                Some(_) => {
                    if attributes.len() >= self.limits.max_attributes_per_element {
                        return Err(self.error(XmlErrorCode::ResourceLimit));
                    }
                    let attribute = self.parse_attribute()?;
                    if attributes
                        .iter()
                        .any(|existing: &XmlAttribute| existing.name == attribute.name)
                    {
                        return Err(self.error(XmlErrorCode::DuplicateAttribute));
                    }
                    attributes
                        .try_reserve(1)
                        .map_err(|_| self.error(XmlErrorCode::AllocationFailed))?;
                    attributes.push(attribute);
                }
                None => return Err(self.error(XmlErrorCode::Malformed)),
            }
        }

        let mut children = Vec::new();
        loop {
            if self.offset == self.input.len() {
                return Err(self.error(XmlErrorCode::Malformed));
            }
            if self.starts_with("</") {
                self.offset += 2;
                let end_name = self.parse_name()?;
                self.skip_whitespace();
                self.expect_byte(b'>')?;
                if end_name != name {
                    return Err(self.error(XmlErrorCode::Malformed));
                }
                break;
            }
            if self.starts_with("<!--") {
                self.skip_comment()?;
                continue;
            }
            if self.starts_with("<?") {
                self.skip_processing_instruction()?;
                continue;
            }
            if self.starts_with("<![CDATA[") {
                let text = self.parse_cdata()?;
                self.push_text(&mut children, text)?;
                continue;
            }
            if self.starts_with("<!DOCTYPE") {
                return Err(self.error(XmlErrorCode::UnsupportedDoctype));
            }
            if self.peek() == Some(b'<') {
                let element = self.parse_element(depth + 1)?;
                children
                    .try_reserve(1)
                    .map_err(|_| self.error(XmlErrorCode::AllocationFailed))?;
                children.push(XmlNode::Element(element));
                continue;
            }
            let text_start = self.offset;
            let text_end = self.find_required("<")?;
            let text = self.decode_text(&self.input[text_start..text_end], text_start)?;
            self.offset = text_end;
            self.push_text(&mut children, text)?;
        }

        Ok(XmlElement {
            name,
            name_offset,
            namespace_uri: None,
            attributes,
            children,
        })
    }

    fn parse_attribute(&mut self) -> Result<XmlAttribute, XmlError> {
        let name_offset = self.offset;
        let name = self.parse_name()?;
        self.skip_whitespace();
        self.expect_byte(b'=')?;
        self.skip_whitespace();
        let quote = match self.peek() {
            Some(b'\'') | Some(b'\"') => self
                .take_byte()
                .ok_or_else(|| self.error(XmlErrorCode::Malformed))?,
            _ => return Err(self.error(XmlErrorCode::Malformed)),
        };
        let value_start = self.offset;
        let value_end = self
            .find_byte(quote)
            .ok_or(self.error(XmlErrorCode::Malformed))?;
        let raw = &self.input[value_start..value_end];
        if raw.contains('<') {
            return Err(XmlError::new(XmlErrorCode::Malformed, value_start));
        }
        let value = self.decode_attribute(raw, value_start)?;
        if value.len() > self.limits.max_attribute_value_bytes {
            return Err(XmlError::new(XmlErrorCode::ResourceLimit, value_start));
        }
        self.offset = value_end + 1;
        Ok(XmlAttribute {
            name,
            name_offset,
            namespace_uri: None,
            value,
        })
    }

    fn parse_cdata(&mut self) -> Result<String, XmlError> {
        self.offset += "<![CDATA[".len();
        let text_start = self.offset;
        let text_end = self
            .find("]]>")
            .ok_or(self.error(XmlErrorCode::Malformed))?;
        let text = self.decode_cdata(&self.input[text_start..text_end], text_start)?;
        self.offset = text_end + 3;
        Ok(text)
    }

    fn skip_comment(&mut self) -> Result<(), XmlError> {
        self.offset += "<!--".len();
        let content_start = self.offset;
        let content_end = self
            .find("-->")
            .ok_or(self.error(XmlErrorCode::Malformed))?;
        let content = &self.input[content_start..content_end];
        validate_xml_characters(content, content_start)?;
        if content.contains("--") {
            return Err(XmlError::new(XmlErrorCode::Malformed, content_start));
        }
        self.offset = content_end + "-->".len();
        Ok(())
    }

    fn skip_processing_instruction(&mut self) -> Result<(), XmlError> {
        self.offset += "<?".len();
        let name = self.parse_name()?;
        if name.eq_ignore_ascii_case("xml") {
            return Err(self.error(XmlErrorCode::Malformed));
        }
        let content_start = self.offset;
        let end = self.find("?>").ok_or(self.error(XmlErrorCode::Malformed))?;
        validate_xml_characters(&self.input[content_start..end], content_start)?;
        self.offset = end + "?>".len();
        Ok(())
    }

    fn parse_name(&mut self) -> Result<String, XmlError> {
        let start = self.offset;
        let first = self.peek().ok_or(self.error(XmlErrorCode::InvalidName))?;
        if !is_name_start(first) {
            return Err(self.error(XmlErrorCode::InvalidName));
        }
        self.offset += 1;
        while let Some(byte) = self.peek() {
            if !is_name_continue(byte) {
                break;
            }
            self.offset += 1;
        }
        let length = self.offset - start;
        if length > self.limits.max_name_bytes {
            return Err(XmlError::new(XmlErrorCode::ResourceLimit, start));
        }
        let mut name = String::new();
        name.try_reserve_exact(length)
            .map_err(|_| XmlError::new(XmlErrorCode::AllocationFailed, start))?;
        name.push_str(&self.input[start..self.offset]);
        Ok(name)
    }

    fn decode_text(&self, raw: &str, raw_offset: usize) -> Result<String, XmlError> {
        self.decode(raw, raw_offset, true, true, self.limits.max_text_bytes)
    }

    fn decode_attribute(&self, raw: &str, raw_offset: usize) -> Result<String, XmlError> {
        self.decode(
            raw,
            raw_offset,
            true,
            false,
            self.limits.max_attribute_value_bytes,
        )
    }

    fn decode_cdata(&self, raw: &str, raw_offset: usize) -> Result<String, XmlError> {
        self.decode(raw, raw_offset, false, false, self.limits.max_text_bytes)
    }

    fn decode(
        &self,
        raw: &str,
        raw_offset: usize,
        entities: bool,
        forbid_cdata_close: bool,
        maximum_bytes: usize,
    ) -> Result<String, XmlError> {
        if raw.len() > maximum_bytes {
            return Err(XmlError::new(XmlErrorCode::ResourceLimit, raw_offset));
        }
        if forbid_cdata_close && raw.contains("]]>") {
            return Err(XmlError::new(XmlErrorCode::Malformed, raw_offset));
        }
        let mut output = String::new();
        output
            .try_reserve(raw.len())
            .map_err(|_| XmlError::new(XmlErrorCode::AllocationFailed, raw_offset))?;
        let mut index = 0;
        while index < raw.len() {
            let remaining = &raw[index..];
            if entities && remaining.starts_with('&') {
                let end = remaining.find(';').ok_or(XmlError::new(
                    XmlErrorCode::UnsupportedEntity,
                    raw_offset + index,
                ))?;
                let reference = &remaining[1..end];
                let character = decode_entity(reference).ok_or(XmlError::new(
                    XmlErrorCode::UnsupportedEntity,
                    raw_offset + index,
                ))?;
                if !is_xml_character(character) {
                    return Err(XmlError::new(
                        XmlErrorCode::InvalidCharacter,
                        raw_offset + index,
                    ));
                }
                push_character(&mut output, character, raw_offset + index)?;
                index += end + 1;
                continue;
            }
            let character = remaining
                .chars()
                .next()
                .ok_or(XmlError::new(XmlErrorCode::Malformed, raw_offset + index))?;
            if !is_xml_character(character) {
                return Err(XmlError::new(
                    XmlErrorCode::InvalidCharacter,
                    raw_offset + index,
                ));
            }
            if character == '\r' {
                output.push('\n');
                index += character.len_utf8();
                if raw[index..].starts_with('\n') {
                    index += '\n'.len_utf8();
                }
            } else {
                push_character(&mut output, character, raw_offset + index)?;
                index += character.len_utf8();
            }
            if output.len() > maximum_bytes {
                return Err(XmlError::new(
                    XmlErrorCode::ResourceLimit,
                    raw_offset + index,
                ));
            }
        }
        Ok(output)
    }

    fn push_text(&mut self, children: &mut Vec<XmlNode>, text: String) -> Result<(), XmlError> {
        if text.is_empty() {
            return Ok(());
        }
        let next = self
            .total_text_bytes
            .checked_add(text.len())
            .ok_or(self.error(XmlErrorCode::ResourceLimit))?;
        if next > self.limits.max_total_text_bytes {
            return Err(self.error(XmlErrorCode::ResourceLimit));
        }
        self.reserve_node()?;
        children
            .try_reserve(1)
            .map_err(|_| self.error(XmlErrorCode::AllocationFailed))?;
        children.push(XmlNode::Text(text));
        self.total_text_bytes = next;
        Ok(())
    }

    fn reserve_node(&mut self) -> Result<(), XmlError> {
        if self.node_count >= self.limits.max_nodes {
            return Err(self.error(XmlErrorCode::ResourceLimit));
        }
        self.node_count += 1;
        Ok(())
    }

    fn find(&self, needle: &str) -> Option<usize> {
        self.input[self.offset..]
            .find(needle)
            .map(|relative| self.offset + relative)
    }

    fn find_required(&self, needle: &str) -> Result<usize, XmlError> {
        self.find(needle).ok_or(self.error(XmlErrorCode::Malformed))
    }

    fn find_byte(&self, byte: u8) -> Option<usize> {
        self.input.as_bytes()[self.offset..]
            .iter()
            .position(|candidate| *candidate == byte)
            .map(|relative| self.offset + relative)
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\r' | b'\n')) {
            self.offset += 1;
        }
    }

    fn starts_with(&self, prefix: &str) -> bool {
        self.input[self.offset..].starts_with(prefix)
    }

    fn expect_byte(&mut self, byte: u8) -> Result<(), XmlError> {
        if self.take_byte() == Some(byte) {
            Ok(())
        } else {
            Err(self.error(XmlErrorCode::Malformed))
        }
    }

    fn take_byte(&mut self) -> Option<u8> {
        let byte = self.peek()?;
        self.offset += 1;
        Some(byte)
    }

    fn peek(&self) -> Option<u8> {
        self.input.as_bytes().get(self.offset).copied()
    }

    fn peek_at(&self, relative: usize) -> Option<u8> {
        self.offset
            .checked_add(relative)
            .and_then(|offset| self.input.as_bytes().get(offset).copied())
    }

    fn error(&self, code: XmlErrorCode) -> XmlError {
        XmlError::new(code, self.offset)
    }
}

fn is_name_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || matches!(byte, b'_' | b':')
}

fn is_name_continue(byte: u8) -> bool {
    is_name_start(byte) || byte.is_ascii_digit() || matches!(byte, b'-' | b'.')
}

fn split_qualified_name(name: &str) -> (Option<&str>, &str) {
    name.split_once(':')
        .map_or((None, name), |(prefix, local)| (Some(prefix), local))
}

fn checked_qualified_name(name: &str, offset: usize) -> Result<(Option<&str>, &str), XmlError> {
    let (prefix, local) = split_qualified_name(name);
    if local.is_empty() || prefix.is_some_and(str::is_empty) || local.contains(':') {
        return Err(XmlError::new(XmlErrorCode::InvalidNamespace, offset));
    }
    Ok((prefix, local))
}

fn resolve_namespaces(root: &mut XmlElement) -> Result<(), XmlError> {
    let mut scope = HashMap::new();
    scope
        .try_reserve(1)
        .map_err(|_| XmlError::new(XmlErrorCode::AllocationFailed, 0))?;
    scope.insert("xml".to_owned(), Arc::<str>::from(XML_NAMESPACE));
    resolve_element_namespaces(root, &scope)
}

fn resolve_element_namespaces(
    element: &mut XmlElement,
    inherited: &HashMap<String, Arc<str>>,
) -> Result<(), XmlError> {
    let mut scope = HashMap::new();
    scope
        .try_reserve(
            inherited
                .len()
                .checked_add(element.attributes.len())
                .ok_or(XmlError::new(XmlErrorCode::ResourceLimit, 0))?,
        )
        .map_err(|_| XmlError::new(XmlErrorCode::AllocationFailed, 0))?;
    scope.extend(
        inherited
            .iter()
            .map(|(prefix, namespace)| (prefix.clone(), Arc::clone(namespace))),
    );

    for attribute in &mut element.attributes {
        let (prefix, local) = checked_qualified_name(&attribute.name, attribute.name_offset)?;
        let declared_prefix = if attribute.name == "xmlns" {
            Some("")
        } else if prefix == Some("xmlns") {
            Some(local)
        } else {
            None
        };
        let Some(declared_prefix) = declared_prefix else {
            continue;
        };
        attribute.namespace_uri = Some(Arc::from(XMLNS_NAMESPACE));
        let namespace = attribute.value.as_str();
        if declared_prefix == "xmlns"
            || namespace == XMLNS_NAMESPACE
            || (declared_prefix == "xml" && namespace != XML_NAMESPACE)
            || (declared_prefix != "xml" && namespace == XML_NAMESPACE)
            || (!declared_prefix.is_empty() && namespace.is_empty())
        {
            return Err(XmlError::new(
                XmlErrorCode::InvalidNamespace,
                attribute.name_offset,
            ));
        }
        if namespace.is_empty() {
            scope.remove("");
        } else {
            scope.insert(declared_prefix.to_owned(), Arc::from(namespace));
        }
    }

    let (prefix, _) = checked_qualified_name(&element.name, element.name_offset)?;
    if prefix == Some("xmlns") {
        return Err(XmlError::new(
            XmlErrorCode::InvalidNamespace,
            element.name_offset,
        ));
    }
    element.namespace_uri = match prefix {
        Some(prefix) => Some(scope.get(prefix).cloned().ok_or(XmlError::new(
            XmlErrorCode::InvalidNamespace,
            element.name_offset,
        ))?),
        None => scope.get("").cloned(),
    };

    let mut expanded_attributes = HashSet::new();
    expanded_attributes
        .try_reserve(element.attributes.len())
        .map_err(|_| XmlError::new(XmlErrorCode::AllocationFailed, 0))?;
    for attribute in &mut element.attributes {
        if attribute.namespace_uri.as_deref() == Some(XMLNS_NAMESPACE) {
            continue;
        }
        let (prefix, local) = checked_qualified_name(&attribute.name, attribute.name_offset)?;
        attribute.namespace_uri = match prefix {
            Some(prefix) => Some(scope.get(prefix).cloned().ok_or(XmlError::new(
                XmlErrorCode::InvalidNamespace,
                attribute.name_offset,
            ))?),
            None => None,
        };
        let expanded = (
            attribute.namespace_uri.as_deref().map(str::to_owned),
            local.to_owned(),
        );
        if !expanded_attributes.insert(expanded) {
            return Err(XmlError::new(
                XmlErrorCode::DuplicateAttribute,
                attribute.name_offset,
            ));
        }
    }

    for child in &mut element.children {
        if let XmlNode::Element(child) = child {
            resolve_element_namespaces(child, &scope)?;
        }
    }
    Ok(())
}

fn validate_xml_declaration(
    content: &str,
    content_offset: usize,
    declaration_offset: usize,
) -> Result<(), XmlError> {
    let bytes = content.as_bytes();
    let mut offset = 0;
    let mut field = 0;
    while offset < bytes.len() {
        let whitespace_start = offset;
        while matches!(bytes.get(offset), Some(b' ' | b'\t' | b'\r' | b'\n')) {
            offset += 1;
        }
        if offset == bytes.len() {
            break;
        }
        if whitespace_start == offset {
            return Err(XmlError::new(
                XmlErrorCode::Malformed,
                content_offset + offset,
            ));
        }
        let name_start = offset;
        while matches!(bytes.get(offset), Some(byte) if byte.is_ascii_alphabetic()) {
            offset += 1;
        }
        let name = &content[name_start..offset];
        while matches!(bytes.get(offset), Some(b' ' | b'\t' | b'\r' | b'\n')) {
            offset += 1;
        }
        if bytes.get(offset) != Some(&b'=') {
            return Err(XmlError::new(
                XmlErrorCode::Malformed,
                content_offset + offset,
            ));
        }
        offset += 1;
        while matches!(bytes.get(offset), Some(b' ' | b'\t' | b'\r' | b'\n')) {
            offset += 1;
        }
        let quote = match bytes.get(offset) {
            Some(b'\'') | Some(b'"') => bytes[offset],
            _ => {
                return Err(XmlError::new(
                    XmlErrorCode::Malformed,
                    content_offset + offset,
                ));
            }
        };
        offset += 1;
        let value_start = offset;
        while bytes.get(offset).is_some_and(|byte| *byte != quote) {
            offset += 1;
        }
        if bytes.get(offset) != Some(&quote) {
            return Err(XmlError::new(
                XmlErrorCode::Malformed,
                content_offset + offset,
            ));
        }
        let value = &content[value_start..offset];
        offset += 1;

        match (field, name) {
            (0, "version") => {
                if value != "1.0" {
                    return Err(XmlError::new(
                        XmlErrorCode::UnsupportedVersion,
                        content_offset + value_start,
                    ));
                }
            }
            (1, "encoding") => {
                if !value.eq_ignore_ascii_case("UTF-8") {
                    return Err(XmlError::new(
                        XmlErrorCode::UnsupportedEncoding,
                        content_offset + value_start,
                    ));
                }
            }
            (1 | 2, "standalone") => {
                if !matches!(value, "yes" | "no") {
                    return Err(XmlError::new(
                        XmlErrorCode::Malformed,
                        content_offset + value_start,
                    ));
                }
                field = 3;
                continue;
            }
            _ => {
                return Err(XmlError::new(
                    XmlErrorCode::Malformed,
                    content_offset + name_start,
                ));
            }
        }
        field += 1;
    }
    if field == 0 {
        return Err(XmlError::new(XmlErrorCode::Malformed, declaration_offset));
    }
    Ok(())
}

fn decode_entity(reference: &str) -> Option<char> {
    match reference {
        "amp" => Some('&'),
        "apos" => Some('\''),
        "gt" => Some('>'),
        "lt" => Some('<'),
        "quot" => Some('\"'),
        _ => decode_numeric_entity(reference),
    }
}

fn decode_numeric_entity(reference: &str) -> Option<char> {
    let digits = reference
        .strip_prefix("#x")
        .or_else(|| reference.strip_prefix("#X"));
    let value = if let Some(digits) = digits {
        u32::from_str_radix(digits, 16).ok()?
    } else {
        reference.strip_prefix('#')?.parse::<u32>().ok()?
    };
    char::from_u32(value)
}

fn is_xml_character(character: char) -> bool {
    matches!(character, '\u{9}' | '\u{A}' | '\u{D}')
        || ('\u{20}'..='\u{D7FF}').contains(&character)
        || ('\u{E000}'..='\u{FFFD}').contains(&character)
        || ('\u{10000}'..='\u{10FFFF}').contains(&character)
}

fn validate_xml_characters(raw: &str, raw_offset: usize) -> Result<(), XmlError> {
    for (relative, character) in raw.char_indices() {
        if !is_xml_character(character) {
            return Err(XmlError::new(
                XmlErrorCode::InvalidCharacter,
                raw_offset + relative,
            ));
        }
    }
    Ok(())
}

fn push_character(output: &mut String, character: char, offset: usize) -> Result<(), XmlError> {
    output
        .try_reserve(character.len_utf8())
        .map_err(|_| XmlError::new(XmlErrorCode::AllocationFailed, offset))?;
    output.push(character);
    Ok(())
}
