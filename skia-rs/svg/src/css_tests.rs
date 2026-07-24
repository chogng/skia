use skia_xml::{XmlDocument, XmlLimits};

use super::Stylesheet;

fn document(source: &str) -> XmlDocument {
    XmlDocument::parse(source.as_bytes(), XmlLimits::default()).expect("XML")
}

#[test]
fn cascade_honors_selectors_specificity_inline_and_important() {
    let xml = document(
        r#"<svg xmlns="http://www.w3.org/2000/svg">
          <style>
            rect { fill: red; stroke: black }
            g > .hot[data-kind~="primary"] { fill: blue }
            #chosen { stroke: green !important }
          </style>
          <g><rect id="chosen" class="hot" data-kind="wide primary"
                   fill="white" style="stroke: yellow; opacity: .5"/></g>
        </svg>"#,
    );
    let root = xml.root();
    let group = root
        .children()
        .iter()
        .filter_map(|child| child.as_element())
        .find(|element| element.local_name() == "g")
        .expect("group");
    let rect = group
        .children()
        .iter()
        .find_map(|child| child.as_element())
        .expect("rect");
    let sheet = Stylesheet::parse(root, 64).expect("stylesheet");
    let cascade = sheet.cascade(rect, &[root, group]).expect("cascade");

    assert_eq!(cascade.property("fill"), Some("blue"));
    assert_eq!(cascade.property("stroke"), Some("green"));
    assert_eq!(cascade.property("opacity"), Some(".5"));
}

#[test]
fn descendant_selector_and_source_order_are_deterministic() {
    let xml = document(
        r#"<svg><style>
          svg .item { fill: red }
          svg g .item { fill: blue }
          svg g .item { fill: green }
        </style><g><path class="item"/></g></svg>"#,
    );
    let root = xml.root();
    let group = root
        .children()
        .iter()
        .filter_map(|child| child.as_element())
        .find(|element| element.local_name() == "g")
        .expect("group");
    let path = group
        .children()
        .iter()
        .find_map(|child| child.as_element())
        .expect("path");
    let sheet = Stylesheet::parse(root, 32).expect("stylesheet");
    let cascade = sheet.cascade(path, &[root, group]).expect("cascade");

    assert_eq!(cascade.property("fill"), Some("green"));
}
