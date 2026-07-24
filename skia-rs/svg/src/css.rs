use std::collections::HashMap;

use skia_xml::{XmlElement, XmlNode};

const SVG_NAMESPACE: &str = "http://www.w3.org/2000/svg";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CssError {
    Invalid,
    ResourceLimit,
    AllocationFailed,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Stylesheet {
    rules: Vec<Rule>,
    maximum_declarations: usize,
}

#[derive(Clone, Debug)]
struct Rule {
    selector: Selector,
    declarations: Vec<Declaration>,
    order: u32,
}

#[derive(Clone, Debug)]
struct Declaration {
    name: String,
    value: String,
    important: bool,
}

#[derive(Clone, Debug)]
struct Selector {
    compounds: Vec<Compound>,
    combinators: Vec<Combinator>,
    specificity: u32,
}

#[derive(Clone, Copy, Debug)]
enum Combinator {
    Descendant,
    Child,
}

#[derive(Clone, Debug, Default)]
struct Compound {
    type_name: Option<String>,
    id: Option<String>,
    classes: Vec<String>,
    attributes: Vec<AttributeSelector>,
    root: bool,
}

#[derive(Clone, Debug)]
struct AttributeSelector {
    name: String,
    operation: AttributeOperation,
    value: Option<String>,
    ascii_case_insensitive: bool,
}

#[derive(Clone, Copy, Debug)]
enum AttributeOperation {
    Exists,
    Equals,
    Includes,
    DashMatch,
    Prefix,
    Suffix,
    Substring,
}

#[derive(Clone, Debug)]
struct WinningDeclaration {
    value: String,
    important: bool,
    origin: u8,
    specificity: u32,
    order: u32,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct CascadedStyle {
    properties: HashMap<String, WinningDeclaration>,
}

impl CascadedStyle {
    pub(crate) fn property(&self, name: &str) -> Option<&str> {
        self.properties.get(name).map(|entry| entry.value.as_str())
    }
}

impl Stylesheet {
    pub(crate) fn parse(root: &XmlElement, maximum_declarations: usize) -> Result<Self, CssError> {
        if maximum_declarations == 0 {
            return Err(CssError::ResourceLimit);
        }
        let mut sources = Vec::new();
        collect_style_sources(root, &mut sources)?;
        let mut rules = Vec::new();
        let mut declaration_count = 0_usize;
        for source in sources {
            parse_stylesheet(
                &source,
                maximum_declarations,
                &mut declaration_count,
                &mut rules,
            )?;
        }
        Ok(Self {
            rules,
            maximum_declarations,
        })
    }

    pub(crate) fn cascade(
        &self,
        element: &XmlElement,
        ancestors: &[&XmlElement],
    ) -> Result<CascadedStyle, CssError> {
        let mut result = CascadedStyle::default();
        for attribute in element.attributes() {
            if attribute.namespace_uri().is_none()
                && !matches!(attribute.local_name(), "style" | "id" | "class")
            {
                result.insert(
                    attribute.local_name(),
                    attribute.value(),
                    false,
                    0,
                    0,
                    0,
                    self.maximum_declarations,
                )?;
            }
        }
        for rule in &self.rules {
            if rule.selector.matches(element, ancestors) {
                for declaration in &rule.declarations {
                    result.insert(
                        &declaration.name,
                        &declaration.value,
                        declaration.important,
                        1,
                        rule.selector.specificity,
                        rule.order,
                        self.maximum_declarations,
                    )?;
                }
            }
        }
        if let Some(inline) = element.attribute_ns(None, "style") {
            let declarations = parse_declarations(inline, self.maximum_declarations)?;
            for (order, declaration) in declarations.iter().enumerate() {
                result.insert(
                    &declaration.name,
                    &declaration.value,
                    declaration.important,
                    2,
                    u32::MAX,
                    u32::try_from(order).unwrap_or(u32::MAX),
                    self.maximum_declarations,
                )?;
            }
        }
        Ok(result)
    }
}

impl CascadedStyle {
    #[allow(clippy::too_many_arguments)]
    fn insert(
        &mut self,
        name: &str,
        value: &str,
        important: bool,
        origin: u8,
        specificity: u32,
        order: u32,
        maximum: usize,
    ) -> Result<(), CssError> {
        let name = name.trim().to_ascii_lowercase();
        if name.is_empty() || value.trim().is_empty() {
            return Err(CssError::Invalid);
        }
        let replace = self.properties.get(&name).is_none_or(|current| {
            (important, origin, specificity, order)
                >= (
                    current.important,
                    current.origin,
                    current.specificity,
                    current.order,
                )
        });
        if !replace {
            return Ok(());
        }
        if !self.properties.contains_key(&name) && self.properties.len() == maximum {
            return Err(CssError::ResourceLimit);
        }
        self.properties
            .try_reserve(1)
            .map_err(|_| CssError::AllocationFailed)?;
        self.properties.insert(
            name,
            WinningDeclaration {
                value: value.trim().to_owned(),
                important,
                origin,
                specificity,
                order,
            },
        );
        Ok(())
    }
}

fn collect_style_sources(element: &XmlElement, output: &mut Vec<String>) -> Result<(), CssError> {
    if element
        .namespace_uri()
        .is_none_or(|uri| uri == SVG_NAMESPACE)
        && element.local_name() == "style"
    {
        if !matches!(element.attribute_ns(None, "type"), None | Some("text/css")) {
            return Err(CssError::Invalid);
        }
        let mut source = String::new();
        for child in element.children() {
            match child {
                XmlNode::Text(text) => source.push_str(text),
                XmlNode::Element(_) => return Err(CssError::Invalid),
            }
        }
        output
            .try_reserve(1)
            .map_err(|_| CssError::AllocationFailed)?;
        output.push(source);
    }
    for child in element.children() {
        if let XmlNode::Element(child) = child {
            collect_style_sources(child, output)?;
        }
    }
    Ok(())
}

fn parse_stylesheet(
    source: &str,
    maximum: usize,
    declaration_count: &mut usize,
    output: &mut Vec<Rule>,
) -> Result<(), CssError> {
    let source = strip_comments(source)?;
    let mut offset = 0_usize;
    while offset < source.len() {
        skip_whitespace(&source, &mut offset);
        if offset == source.len() {
            break;
        }
        if source.as_bytes()[offset] == b'@' {
            return Err(CssError::Invalid);
        }
        let open = find_top_level(&source, offset, b'{')?.ok_or(CssError::Invalid)?;
        let close = find_matching_brace(&source, open)?;
        let selector_source = source[offset..open].trim();
        let declarations = parse_declarations(&source[open + 1..close], maximum)?;
        *declaration_count = declaration_count
            .checked_add(declarations.len())
            .ok_or(CssError::ResourceLimit)?;
        if *declaration_count > maximum {
            return Err(CssError::ResourceLimit);
        }
        let selectors = split_top_level(selector_source, b',')?;
        for selector_source in selectors {
            if output.len() == maximum {
                return Err(CssError::ResourceLimit);
            }
            output
                .try_reserve(1)
                .map_err(|_| CssError::AllocationFailed)?;
            output.push(Rule {
                selector: parse_selector(selector_source.trim())?,
                declarations: declarations.clone(),
                order: u32::try_from(output.len()).unwrap_or(u32::MAX),
            });
        }
        offset = close + 1;
    }
    Ok(())
}

fn strip_comments(source: &str) -> Result<String, CssError> {
    let mut output = String::new();
    output
        .try_reserve(source.len())
        .map_err(|_| CssError::AllocationFailed)?;
    let mut offset = 0_usize;
    while offset < source.len() {
        if source[offset..].starts_with("/*") {
            let end = source[offset + 2..]
                .find("*/")
                .map(|value| offset + 2 + value)
                .ok_or(CssError::Invalid)?;
            output.push(' ');
            offset = end + 2;
        } else {
            let ch = source[offset..].chars().next().ok_or(CssError::Invalid)?;
            output.push(ch);
            offset += ch.len_utf8();
        }
    }
    Ok(output)
}

fn parse_declarations(source: &str, maximum: usize) -> Result<Vec<Declaration>, CssError> {
    let mut declarations = Vec::new();
    for declaration in split_top_level(source, b';')? {
        let declaration = declaration.trim();
        if declaration.is_empty() {
            continue;
        }
        if declarations.len() == maximum {
            return Err(CssError::ResourceLimit);
        }
        let colon = find_top_level(declaration, 0, b':')?.ok_or(CssError::Invalid)?;
        let name = declaration[..colon].trim();
        if !is_property_name(name) {
            return Err(CssError::Invalid);
        }
        let mut value = declaration[colon + 1..].trim();
        let important = value
            .to_ascii_lowercase()
            .strip_suffix("!important")
            .map(str::trim)
            .map(|trimmed| {
                let length = trimmed.len();
                value = &value[..length];
                true
            })
            .unwrap_or(false);
        if value.is_empty() {
            return Err(CssError::Invalid);
        }
        declarations
            .try_reserve(1)
            .map_err(|_| CssError::AllocationFailed)?;
        declarations.push(Declaration {
            name: name.to_ascii_lowercase(),
            value: value.to_owned(),
            important,
        });
    }
    Ok(declarations)
}

fn parse_selector(source: &str) -> Result<Selector, CssError> {
    if source.is_empty() {
        return Err(CssError::Invalid);
    }
    let mut parser = SelectorParser { source, offset: 0 };
    let mut compounds = Vec::new();
    let mut combinators = Vec::new();
    let mut specificity = 0_u32;
    loop {
        let compound = parser.compound(&mut specificity)?;
        compounds.push(compound);
        let whitespace = parser.skip_whitespace();
        if parser.offset == source.len() {
            break;
        }
        if parser.consume(b'>') {
            parser.skip_whitespace();
            combinators.push(Combinator::Child);
        } else if whitespace {
            combinators.push(Combinator::Descendant);
        } else {
            return Err(CssError::Invalid);
        }
    }
    if compounds.len() != combinators.len() + 1 {
        return Err(CssError::Invalid);
    }
    Ok(Selector {
        compounds,
        combinators,
        specificity,
    })
}

impl Selector {
    fn matches(&self, element: &XmlElement, ancestors: &[&XmlElement]) -> bool {
        let last = self.compounds.len() - 1;
        if !self.compounds[last].matches(element, ancestors.is_empty()) {
            return false;
        }
        let mut ancestor_end = ancestors.len();
        for index in (0..last).rev() {
            match self.combinators[index] {
                Combinator::Child => {
                    if ancestor_end == 0 {
                        return false;
                    }
                    ancestor_end -= 1;
                    if !self.compounds[index].matches(ancestors[ancestor_end], ancestor_end == 0) {
                        return false;
                    }
                }
                Combinator::Descendant => {
                    let Some(found) = (0..ancestor_end).rev().find(|candidate| {
                        self.compounds[index].matches(ancestors[*candidate], *candidate == 0)
                    }) else {
                        return false;
                    };
                    ancestor_end = found;
                }
            }
        }
        true
    }
}

impl Compound {
    fn matches(&self, element: &XmlElement, is_root: bool) -> bool {
        if self
            .type_name
            .as_ref()
            .is_some_and(|name| name != element.local_name())
            || self
                .id
                .as_ref()
                .is_some_and(|id| element.attribute_ns(None, "id") != Some(id.as_str()))
            || self.root && !is_root
        {
            return false;
        }
        let classes = element
            .attribute_ns(None, "class")
            .unwrap_or_default()
            .split_ascii_whitespace()
            .collect::<Vec<_>>();
        if self
            .classes
            .iter()
            .any(|class| !classes.contains(&class.as_str()))
        {
            return false;
        }
        self.attributes
            .iter()
            .all(|selector| selector.matches(element))
    }
}

impl AttributeSelector {
    fn matches(&self, element: &XmlElement) -> bool {
        let Some(actual) = element.attribute_ns(None, &self.name) else {
            return false;
        };
        if matches!(self.operation, AttributeOperation::Exists) {
            return true;
        }
        let expected = self.value.as_deref().unwrap_or_default();
        let compare = |left: &str, right: &str| {
            if self.ascii_case_insensitive {
                left.eq_ignore_ascii_case(right)
            } else {
                left == right
            }
        };
        match self.operation {
            AttributeOperation::Exists => true,
            AttributeOperation::Equals => compare(actual, expected),
            AttributeOperation::Includes => actual
                .split_ascii_whitespace()
                .any(|part| compare(part, expected)),
            AttributeOperation::DashMatch => {
                compare(actual, expected)
                    || actual
                        .strip_prefix(expected)
                        .is_some_and(|suffix| suffix.starts_with('-'))
            }
            AttributeOperation::Prefix => {
                if self.ascii_case_insensitive {
                    actual
                        .get(..expected.len())
                        .is_some_and(|part| part.eq_ignore_ascii_case(expected))
                } else {
                    actual.starts_with(expected)
                }
            }
            AttributeOperation::Suffix => {
                if self.ascii_case_insensitive {
                    actual
                        .get(actual.len().saturating_sub(expected.len())..)
                        .is_some_and(|part| part.eq_ignore_ascii_case(expected))
                } else {
                    actual.ends_with(expected)
                }
            }
            AttributeOperation::Substring => {
                if self.ascii_case_insensitive {
                    actual
                        .to_ascii_lowercase()
                        .contains(&expected.to_ascii_lowercase())
                } else {
                    actual.contains(expected)
                }
            }
        }
    }
}

struct SelectorParser<'a> {
    source: &'a str,
    offset: usize,
}

impl SelectorParser<'_> {
    fn compound(&mut self, specificity: &mut u32) -> Result<Compound, CssError> {
        let mut compound = Compound::default();
        let universal = self.consume(b'*');
        if universal {
            // Universal selectors do not affect specificity.
        } else if self.peek().is_some_and(is_identifier_start) {
            compound.type_name = Some(self.identifier()?);
            *specificity = specificity.saturating_add(1);
        }
        let mut consumed = universal || compound.type_name.is_some();
        loop {
            match self.peek() {
                Some(b'#') => {
                    self.offset += 1;
                    if compound.id.replace(self.identifier()?).is_some() {
                        return Err(CssError::Invalid);
                    }
                    *specificity = specificity.saturating_add(1 << 16);
                    consumed = true;
                }
                Some(b'.') => {
                    self.offset += 1;
                    compound.classes.push(self.identifier()?);
                    *specificity = specificity.saturating_add(1 << 8);
                    consumed = true;
                }
                Some(b'[') => {
                    compound.attributes.push(self.attribute()?);
                    *specificity = specificity.saturating_add(1 << 8);
                    consumed = true;
                }
                Some(b':') => {
                    self.offset += 1;
                    let pseudo = self.identifier()?;
                    if pseudo != "root" {
                        return Err(CssError::Invalid);
                    }
                    compound.root = true;
                    *specificity = specificity.saturating_add(1 << 8);
                    consumed = true;
                }
                _ => break,
            }
        }
        if !consumed {
            return Err(CssError::Invalid);
        }
        Ok(compound)
    }

    fn attribute(&mut self) -> Result<AttributeSelector, CssError> {
        if !self.consume(b'[') {
            return Err(CssError::Invalid);
        }
        self.skip_whitespace();
        let name = self.identifier()?;
        self.skip_whitespace();
        if self.consume(b']') {
            return Ok(AttributeSelector {
                name,
                operation: AttributeOperation::Exists,
                value: None,
                ascii_case_insensitive: false,
            });
        }
        let operation = if self.consume_bytes(b"~=") {
            AttributeOperation::Includes
        } else if self.consume_bytes(b"|=") {
            AttributeOperation::DashMatch
        } else if self.consume_bytes(b"^=") {
            AttributeOperation::Prefix
        } else if self.consume_bytes(b"$=") {
            AttributeOperation::Suffix
        } else if self.consume_bytes(b"*=") {
            AttributeOperation::Substring
        } else if self.consume(b'=') {
            AttributeOperation::Equals
        } else {
            return Err(CssError::Invalid);
        };
        self.skip_whitespace();
        let value = if matches!(self.peek(), Some(b'"' | b'\'')) {
            self.quoted()?
        } else {
            self.identifier()?
        };
        self.skip_whitespace();
        let ascii_case_insensitive = if self
            .peek()
            .is_some_and(|byte| byte.eq_ignore_ascii_case(&b'i'))
        {
            self.offset += 1;
            self.skip_whitespace();
            true
        } else if self
            .peek()
            .is_some_and(|byte| byte.eq_ignore_ascii_case(&b's'))
        {
            self.offset += 1;
            self.skip_whitespace();
            false
        } else {
            false
        };
        if !self.consume(b']') {
            return Err(CssError::Invalid);
        }
        Ok(AttributeSelector {
            name,
            operation,
            value: Some(value),
            ascii_case_insensitive,
        })
    }

    fn identifier(&mut self) -> Result<String, CssError> {
        let start = self.offset;
        if self.consume(b'-') && !self.peek().is_some_and(is_identifier_start) {
            return Err(CssError::Invalid);
        }
        if self.offset == start && !self.peek().is_some_and(is_identifier_start) {
            return Err(CssError::Invalid);
        }
        self.offset += 1;
        while self.peek().is_some_and(is_identifier_continue) {
            self.offset += 1;
        }
        Ok(self.source[start..self.offset].to_owned())
    }

    fn quoted(&mut self) -> Result<String, CssError> {
        let quote = self.peek().ok_or(CssError::Invalid)?;
        self.offset += 1;
        let start = self.offset;
        while let Some(byte) = self.peek() {
            if byte == quote {
                let value = self.source[start..self.offset].to_owned();
                self.offset += 1;
                return Ok(value);
            }
            if byte == b'\\' || byte == b'\n' || byte == b'\r' {
                return Err(CssError::Invalid);
            }
            self.offset += 1;
        }
        Err(CssError::Invalid)
    }

    fn skip_whitespace(&mut self) -> bool {
        let start = self.offset;
        while self.peek().is_some_and(|byte| byte.is_ascii_whitespace()) {
            self.offset += 1;
        }
        self.offset != start
    }

    fn consume(&mut self, requested: u8) -> bool {
        if self.peek() == Some(requested) {
            self.offset += 1;
            true
        } else {
            false
        }
    }

    fn consume_bytes(&mut self, requested: &[u8]) -> bool {
        if self.source.as_bytes()[self.offset..].starts_with(requested) {
            self.offset += requested.len();
            true
        } else {
            false
        }
    }

    fn peek(&self) -> Option<u8> {
        self.source.as_bytes().get(self.offset).copied()
    }
}

fn is_identifier_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_'
}

fn is_identifier_continue(byte: u8) -> bool {
    is_identifier_start(byte) || byte.is_ascii_digit() || byte == b'-'
}

fn is_property_name(value: &str) -> bool {
    let mut bytes = value.bytes();
    bytes
        .next()
        .is_some_and(|byte| byte == b'-' || is_identifier_start(byte))
        && bytes.all(is_identifier_continue)
}

fn skip_whitespace(source: &str, offset: &mut usize) {
    while source
        .as_bytes()
        .get(*offset)
        .is_some_and(u8::is_ascii_whitespace)
    {
        *offset += 1;
    }
}

fn split_top_level(source: &str, delimiter: u8) -> Result<Vec<&str>, CssError> {
    let mut output = Vec::new();
    let mut start = 0_usize;
    let mut quote = None;
    let mut parentheses = 0_u32;
    let bytes = source.as_bytes();
    for (index, byte) in bytes.iter().copied().enumerate() {
        if let Some(active) = quote {
            if byte == b'\\' {
                return Err(CssError::Invalid);
            }
            if byte == active {
                quote = None;
            }
            continue;
        }
        match byte {
            b'"' | b'\'' => quote = Some(byte),
            b'(' => parentheses = parentheses.checked_add(1).ok_or(CssError::Invalid)?,
            b')' => parentheses = parentheses.checked_sub(1).ok_or(CssError::Invalid)?,
            _ if byte == delimiter && parentheses == 0 => {
                output.push(&source[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    if quote.is_some() || parentheses != 0 {
        return Err(CssError::Invalid);
    }
    output.push(&source[start..]);
    Ok(output)
}

fn find_top_level(source: &str, start: usize, requested: u8) -> Result<Option<usize>, CssError> {
    let mut quote = None;
    let mut parentheses = 0_u32;
    for (relative, byte) in source.as_bytes()[start..].iter().copied().enumerate() {
        let index = start + relative;
        if let Some(active) = quote {
            if byte == b'\\' {
                return Err(CssError::Invalid);
            }
            if byte == active {
                quote = None;
            }
            continue;
        }
        match byte {
            b'"' | b'\'' => quote = Some(byte),
            b'(' => parentheses = parentheses.checked_add(1).ok_or(CssError::Invalid)?,
            b')' => parentheses = parentheses.checked_sub(1).ok_or(CssError::Invalid)?,
            _ if byte == requested && parentheses == 0 => return Ok(Some(index)),
            _ => {}
        }
    }
    if quote.is_some() || parentheses != 0 {
        return Err(CssError::Invalid);
    }
    Ok(None)
}

fn find_matching_brace(source: &str, open: usize) -> Result<usize, CssError> {
    let mut quote = None;
    let mut parentheses = 0_u32;
    for (relative, byte) in source.as_bytes()[open + 1..].iter().copied().enumerate() {
        let index = open + 1 + relative;
        if let Some(active) = quote {
            if byte == b'\\' {
                return Err(CssError::Invalid);
            }
            if byte == active {
                quote = None;
            }
            continue;
        }
        match byte {
            b'"' | b'\'' => quote = Some(byte),
            b'(' => parentheses = parentheses.checked_add(1).ok_or(CssError::Invalid)?,
            b')' => parentheses = parentheses.checked_sub(1).ok_or(CssError::Invalid)?,
            b'}' if parentheses == 0 => return Ok(index),
            b'{' if parentheses == 0 => return Err(CssError::Invalid),
            _ => {}
        }
    }
    Err(CssError::Invalid)
}

#[cfg(test)]
#[path = "css_tests.rs"]
mod tests;
