use super::{XmlDocument, XmlErrorCode, XmlLimits, XmlNode};

fn parse(input: &str) -> XmlDocument {
    XmlDocument::parse(input.as_bytes(), XmlLimits::default()).expect("XML document")
}

#[test]
fn parser_retains_nested_content_and_decodes_supported_xml_constructs() {
    let document = parse(
        "<?xml version=\"1.0\"?>\n<!-- metadata -->\n<svg:root xmlns:svg=\"urn:test\" \
         label=\"A &amp; B\"><item><![CDATA[left & right]]></item><?ignored data?><item>\r\nnext \
         &#x2603;</item></svg:root>",
    );
    let root = document.root();
    assert_eq!(root.name(), "svg:root");
    assert_eq!(root.prefix(), Some("svg"));
    assert_eq!(root.local_name(), "root");
    assert_eq!(root.namespace_uri(), Some("urn:test"));
    assert_eq!(root.attribute("label"), Some("A & B"));
    assert_eq!(root.attribute_ns(None, "label"), Some("A & B"));
    assert_eq!(root.attribute("xmlns:svg"), Some("urn:test"));
    assert_eq!(root.children().len(), 2);

    let first = root.children()[0].as_element().expect("first element");
    assert_eq!(first.name(), "item");
    assert_eq!(first.children(), [XmlNode::Text("left & right".to_owned())]);

    let second = root.children()[1].as_element().expect("second element");
    assert_eq!(second.children(), [XmlNode::Text("\nnext ☃".to_owned())]);
}

#[test]
fn parser_resolves_default_prefixed_and_attribute_namespaces() {
    let document = parse(
        r#"<root xmlns="urn:default" xmlns:p="urn:property" p:value="one" value="two"
             xml:lang="en"><child xmlns=""><p:item/></child></root>"#,
    );
    let root = document.root();
    assert_eq!(root.namespace_uri(), Some("urn:default"));
    assert_eq!(
        root.attribute_ns(Some("urn:property"), "value"),
        Some("one")
    );
    assert_eq!(root.attribute_ns(None, "value"), Some("two"));
    assert_eq!(
        root.attribute_ns(Some("http://www.w3.org/XML/1998/namespace"), "lang"),
        Some("en")
    );
    let child = root.children()[0].as_element().expect("child");
    assert_eq!(child.namespace_uri(), None);
    let item = child.children()[0].as_element().expect("prefixed item");
    assert_eq!(item.namespace_uri(), Some("urn:property"));
}

#[test]
fn parser_rejects_invalid_namespace_bindings_and_expanded_duplicates() {
    for input in [
        "<p:root/>",
        r#"<root xmlns:xml="urn:not-xml"/>"#,
        r#"<root xmlns:p=""/>"#,
        r#"<root xmlns:p="urn:same" xmlns:q="urn:same" p:a="1" q:a="2"/>"#,
        r#"<xmlns:root xmlns:xmlns="urn:bad"/>"#,
    ] {
        let error = XmlDocument::parse(input.as_bytes(), XmlLimits::default())
            .expect_err("invalid namespace");
        assert!(
            matches!(
                error.code(),
                XmlErrorCode::InvalidNamespace | XmlErrorCode::DuplicateAttribute
            ),
            "{input}"
        );
    }

    let unresolved =
        XmlDocument::parse(b"<p:root/>", XmlLimits::default()).expect_err("unresolved prefix");
    assert_eq!(unresolved.offset(), 1);

    let plain = parse("<root/>");
    let shifted = parse("<!--prefix--><root/>");
    assert_eq!(plain, shifted);
}

#[test]
fn parser_validates_the_utf8_xml_declaration() {
    let document = parse(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><root/>"#);
    assert_eq!(document.root().name(), "root");

    for (input, code) in [
        (
            r#"<?xml version="1.1"?><root/>"#,
            XmlErrorCode::UnsupportedVersion,
        ),
        (
            r#"<?xml version="1.0" encoding="UTF-16"?><root/>"#,
            XmlErrorCode::UnsupportedEncoding,
        ),
        (
            r#"<root><?xml version="1.0"?></root>"#,
            XmlErrorCode::Malformed,
        ),
        (
            r#"<!--before--><?xml version="1.0"?><root/>"#,
            XmlErrorCode::Malformed,
        ),
    ] {
        let error = XmlDocument::parse(input.as_bytes(), XmlLimits::default())
            .expect_err("invalid XML declaration");
        assert_eq!(error.code(), code, "{input}");
    }
}

#[test]
fn parser_rejects_entity_and_document_forms_that_can_expand_or_escape_the_boundary() {
    for (input, expected) in [
        ("<!DOCTYPE svg><svg/>", XmlErrorCode::UnsupportedDoctype),
        ("<svg>&custom;</svg>", XmlErrorCode::UnsupportedEntity),
        ("<svg a=\"1\" a=\"2\"/>", XmlErrorCode::DuplicateAttribute),
        ("<svg><g></svg>", XmlErrorCode::Malformed),
        ("<svg/>trailing", XmlErrorCode::InvalidDocument),
    ] {
        let error = XmlDocument::parse(input.as_bytes(), XmlLimits::default())
            .expect_err("unsafe or malformed XML is rejected");
        assert_eq!(error.code(), expected, "{input}");
    }
}

#[test]
fn parser_enforces_tree_and_decoded_content_budgets_before_retaining_the_document() {
    let depth_limits = XmlLimits {
        max_depth: 2,
        ..XmlLimits::default()
    };
    let error = XmlDocument::parse(b"<a><b><c/></b></a>", depth_limits).expect_err("depth ceiling");
    assert_eq!(error.code(), XmlErrorCode::ResourceLimit);

    let text_limits = XmlLimits {
        max_total_text_bytes: 3,
        ..XmlLimits::default()
    };
    let error =
        XmlDocument::parse(b"<a>ab<b/>cd</a>", text_limits).expect_err("aggregate text ceiling");
    assert_eq!(error.code(), XmlErrorCode::ResourceLimit);

    let attribute_limits = XmlLimits {
        max_attributes_per_element: 1,
        ..XmlLimits::default()
    };
    let error = XmlDocument::parse(b"<a first=\"1\" second=\"2\"/>", attribute_limits)
        .expect_err("attribute ceiling");
    assert_eq!(error.code(), XmlErrorCode::ResourceLimit);

    let independent_limits = XmlLimits {
        max_text_bytes: 1,
        max_attribute_value_bytes: 4,
        ..XmlLimits::default()
    };
    let document = XmlDocument::parse(b"<a value=\"four\">x</a>", independent_limits)
        .expect("independent attribute limit");
    assert_eq!(document.root().attribute("value"), Some("four"));
}

#[test]
fn parser_reports_invalid_utf8_without_replacing_or_lossily_decoding_source_bytes() {
    let error =
        XmlDocument::parse(b"<a>\xFF</a>", XmlLimits::default()).expect_err("invalid UTF-8");
    assert_eq!(error.code(), XmlErrorCode::InvalidUtf8);
    assert_eq!(error.offset(), 3);
}
