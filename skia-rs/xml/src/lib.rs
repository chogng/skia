//! Bounded, dependency-free XML document parsing.
//!
//! This crate owns a deliberately restricted XML tree model for format
//! adapters. It accepts UTF-8 XML with elements, attributes, text, CDATA,
//! comments, processing instructions, a validated UTF-8 XML 1.0 declaration,
//! and namespace expansion, while rejecting DTDs and arbitrary entities.
//! Callers supply explicit resource ceilings before parsing untrusted bytes.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod xml;

pub use xml::{XmlAttribute, XmlDocument, XmlElement, XmlError, XmlErrorCode, XmlLimits, XmlNode};

#[cfg(test)]
#[path = "xml_tests.rs"]
mod tests;
