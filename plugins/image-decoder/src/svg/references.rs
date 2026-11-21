//! Bounded expansion of references, before usvg converts the document.
//!
//! usvg copies the target of every `use` into its tree, converts marker content once per
//! vertex of each shape that refers to it, and converts or draws pattern, clip-path, mask
//! and filter content once per element that refers to it (`feImage` filter inputs
//! included). This pass walks the document the same way and counts the instantiated
//! elements, the attribute and text bytes the converter parses for them, and the style
//! rules it checks for them. A small file therefore cannot make the converter build an
//! exponential tree. The pass also refuses reference cycles and nesting beyond a depth
//! limit, and names content that is skipped instead of drawn (see `Omission`).
//!
//! The pass over-approximates where usvg shares work (cached clip paths, a cascade that
//! picks one of several declarations): it charges every candidate.
use super::{
    Omission,
    document::{SVG_NAMESPACE, limit},
};
use resvg::usvg::roxmltree::{Document, Node};
use std::{
    collections::{BTreeSet, HashMap},
    io,
    rc::Rc,
};

/// Deepest nesting, counting elements and the references entered on the way.
pub(super) const MAX_DEPTH: usize = 64;
/// Most memory for the converted tree after expanding references, estimated per converted
/// element (counting each reference to shared content and each marker vertex again) and
/// per byte of attributes and text parsed for them. Measured with resvg 0.48.1: about 0.6
/// to 0.9 KiB per element and 3.5 bytes per byte of path data, rounded up here. So at most
/// 262,144 elements, or 64 MiB of attribute and text data.
pub(super) const MAX_TREE_BYTES: u64 = 256 * 1024 * 1024;
const TREE_BYTES_PER_ELEMENT: u64 = 1024;
const TREE_BYTES_PER_DATA_BYTE: u64 = 4;
/// Most style rule checks: rules in the document's style sheets times converted elements.
pub(super) const MAX_RULE_CHECKS: u64 = 1 << 27;

/// What the expansion found.
pub(super) struct Census {
    pub(super) omissions: BTreeSet<Omission>,
    /// The document has text, so the converter needs fonts.
    pub(super) text: bool,
    /// Elements, and their attribute and text bytes, after expanding references.
    pub(super) instances: u64,
    pub(super) data: u64,
}

impl Census {
    /// Estimated memory of the converted tree.
    pub(super) fn tree_bytes(&self) -> u64 {
        tree_bytes(self.instances, self.data)
    }
}

fn tree_bytes(instances: u64, data: u64) -> u64 {
    instances
        .saturating_mul(TREE_BYTES_PER_ELEMENT)
        .saturating_add(data.saturating_mul(TREE_BYTES_PER_DATA_BYTE))
}

/// Walk the document as usvg instantiates it. Fails with the exact bound that is exceeded.
pub(super) fn check(document: &Document<'_>) -> io::Result<Census> {
    let mut walk = Walk::new(document)?;
    let context = walk.blank();
    walk.visit(document.root_element(), &context, Reached::Structure, 0)?;
    let checks = walk.instances.saturating_mul(walk.rule_count);
    if checks > MAX_RULE_CHECKS {
        return Err(limit(format!(
            "This SVG needs more than {MAX_RULE_CHECKS} style rule checks (rules times drawn \
             elements), the limit for SVG images."
        )));
    }
    Ok(Census {
        omissions: walk.omissions,
        text: walk.text,
        instances: walk.instances,
        data: walk.data,
    })
}

/// Presentation properties whose values can refer to elements. Inherited ones come first.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Property {
    Fill,
    Stroke,
    MarkerStart,
    MarkerMid,
    MarkerEnd,
    ClipPath,
    Mask,
    Filter,
}

const PROPERTIES: usize = 8;
const INHERITED: usize = 5;

fn properties(name: &str) -> &'static [Property] {
    use Property::*;
    match name {
        "fill" => &[Fill],
        "stroke" => &[Stroke],
        "marker-start" => &[MarkerStart],
        "marker-mid" => &[MarkerMid],
        "marker-end" => &[MarkerEnd],
        "marker" => &[MarkerStart, MarkerMid, MarkerEnd],
        "clip-path" => &[ClipPath],
        "mask" => &[Mask],
        "filter" => &[Filter],
        _ => &[],
    }
}

/// One element's declared reference values: presentation attributes, matching style rules
/// and its `style` attribute. Every declaration's references count.
#[derive(Default)]
struct Declared<'a> {
    targets: [Vec<&'a str>; PROPERTIES],
    /// The property is declared, so it does not inherit unless a value says so.
    declared: [bool; PROPERTIES],
    /// A value is `inherit`.
    inherit: [bool; PROPERTIES],
    /// A value is `context-fill` or `context-stroke` (SVG 2): the paint of the element
    /// that refers to a marker or `use` content.
    context: [bool; PROPERTIES],
}

impl<'a> Declared<'a> {
    /// Record one declaration; returns whether it concerns a reference property.
    fn add(&mut self, name: &str, value: &'a str, omissions: &mut BTreeSet<Omission>) -> bool {
        let properties = properties(name.trim());
        let value = value.trim();
        for &property in properties {
            let index = property as usize;
            self.declared[index] = true;
            match value {
                "inherit" => self.inherit[index] = true,
                "context-fill" | "context-stroke" => self.context[index] = true,
                _ => {
                    for url in urls(value) {
                        match url {
                            Some(id) => self.targets[index].push(id),
                            None => {
                                omissions.insert(Omission::OtherFiles);
                            }
                        }
                    }
                }
            }
        }
        !properties.is_empty()
    }
}

/// The `url(...)` references in a value: `Some(id)` for a local `#id`, `None` for one
/// into another file. Matching is case-insensitive, like CSS functions.
fn urls(value: &str) -> Vec<Option<&str>> {
    let mut found = Vec::new();
    let mut rest = value;
    while let Some(start) = rest
        .as_bytes()
        .windows(4)
        .position(|window| window.eq_ignore_ascii_case(b"url("))
    {
        let inner = &rest[start + 4..];
        let end = inner.find(')').unwrap_or(inner.len());
        let target = inner[..end]
            .trim()
            .trim_matches(|character| character == '"' || character == '\'')
            .trim();
        rest = &inner[end..];
        if !target.is_empty() {
            found.push(target.strip_prefix('#'));
        }
    }
    found
}

/// Reference values that descendants see: the effective values of inherited properties,
/// the element's own values of the others (for a child's `inherit`), and the paint of the
/// element whose marker or `use` content this is (for `context-fill`).
#[derive(Clone)]
struct Context<'a> {
    values: [Rc<[&'a str]>; PROPERTIES],
    paint: Rc<[&'a str]>,
}

/// How a walk reached an element.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Reached {
    /// As a child of a drawn element: definitions are not drawn there.
    Structure,
    /// As the target of a `use`, `feImage` or another reference that draws it.
    Reference,
}

/// What a property reference needs at its target.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Use {
    Paint,
    Marker,
    Effect(Property),
}

struct Walk<'a, 'input> {
    document: &'a Document<'input>,
    ids: HashMap<&'a str, Vec<Node<'a, 'input>>>,
    /// Style rules that declare a reference property, with those declarations.
    rules: Vec<(simplecss::Selector<'a>, Vec<(&'a str, &'a str)>)>,
    rule_count: u64,
    declared: Vec<Option<Rc<Declared<'a>>>>,
    ancestors: HashMap<usize, Context<'a>>,
    active: Vec<bool>,
    nothing: Rc<Declared<'a>>,
    empty: Rc<[&'a str]>,
    omissions: BTreeSet<Omission>,
    text: bool,
    instances: u64,
    data: u64,
}

impl<'a, 'input> Walk<'a, 'input> {
    /// One pass over every node: nesting depth, IDs, style rules, scripts and text.
    fn new(document: &'a Document<'input>) -> io::Result<Self> {
        let count = document.descendants().count();
        let mut depth = vec![0u32; count];
        let mut ids: HashMap<&'a str, Vec<Node<'a, 'input>>> = HashMap::new();
        let mut sheet = simplecss::StyleSheet::new();
        let mut omissions = BTreeSet::new();
        let mut text = false;
        for node in document.descendants().filter(Node::is_element) {
            let level = node
                .parent_element()
                .map_or(0, |parent| depth[parent.id().get_usize()])
                + 1;
            depth[node.id().get_usize()] = level;
            if level as usize > MAX_DEPTH {
                return Err(depth_limit(document, node));
            }
            for attribute in node.attributes() {
                if attribute.name() == "id" {
                    ids.entry(attribute.value()).or_default().push(node);
                }
            }
            let svg = node
                .tag_name()
                .namespace()
                .is_none_or(|namespace| namespace == SVG_NAMESPACE);
            let name = node.tag_name().name();
            // usvg reads every element named `style`, whatever its namespace.
            if name == "style"
                && node.attribute("type").is_none_or(|kind| kind == "text/css")
                && let Some(css) = node.text()
            {
                sheet.parse_more(css);
            }
            if !svg {
                continue;
            }
            // Scripts and event handlers never run; say so when a document has them.
            if name == "script"
                || node.attributes().any(|attribute| {
                    attribute.namespace().is_none() && attribute.name().starts_with("on")
                })
            {
                omissions.insert(Omission::Scripts);
            }
            text |= name == "text";
        }
        let rule_count = sheet.rules.len() as u64;
        let mut rules = Vec::new();
        for rule in sheet.rules {
            let declarations: Vec<_> = rule
                .declarations
                .iter()
                .filter(|declaration| !properties(declaration.name).is_empty())
                .map(|declaration| (declaration.name, declaration.value))
                .collect();
            if !declarations.is_empty() {
                rules.push((rule.selector, declarations));
            }
        }
        let empty: Rc<[&'a str]> = Rc::from(Vec::new());
        Ok(Self {
            document,
            ids,
            rules,
            rule_count,
            declared: vec![None; count],
            ancestors: HashMap::new(),
            active: vec![false; count],
            nothing: Rc::default(),
            empty,
            omissions,
            text,
            instances: 0,
            data: 0,
        })
    }

    fn blank(&self) -> Context<'a> {
        Context {
            values: std::array::from_fn(|_| self.empty.clone()),
            paint: self.empty.clone(),
        }
    }

    /// Instantiate `node` and the subtree that the converter instantiates with it.
    fn visit(
        &mut self,
        node: Node<'a, 'input>,
        parent: &Context<'a>,
        reached: Reached,
        depth: usize,
    ) -> io::Result<()> {
        if depth > MAX_DEPTH {
            return Err(depth_limit(self.document, node));
        }
        // usvg drops elements outside the SVG namespace and unknown ones, with their
        // content; `foreignObject` is one of them.
        let Some(name) = known(node) else {
            return Ok(());
        };
        self.charge(1, data_bytes(node, name))?;
        if reached == Reached::Structure && definition(name) {
            return Ok(());
        }
        let index = node.id().get_usize();
        self.active[index] = true;
        let own = self.context(node, parent);
        let none = self.empty.clone();
        if shape(name) || text_content(name) {
            for property in [Property::Fill, Property::Stroke] {
                for &id in own.values[property as usize].iter() {
                    self.reference(node, id, Use::Paint, depth, 1, &none)?;
                }
            }
        }
        if shape(name) {
            let vertices = vertices(node, name);
            // Marker content may paint with this shape's fill and stroke.
            let paint = self.paint_of(&own);
            for (property, count) in [
                (Property::MarkerStart, 1),
                (Property::MarkerMid, vertices),
                (Property::MarkerEnd, 1),
            ] {
                for &id in own.values[property as usize].iter() {
                    self.reference(node, id, Use::Marker, depth, count, &paint)?;
                }
            }
        }
        for property in [Property::ClipPath, Property::Mask, Property::Filter] {
            for &id in own.values[property as usize].iter() {
                self.reference(node, id, Use::Effect(property), depth, 1, &none)?;
            }
        }
        match (name, href(node)) {
            ("use", Some(Href::Local(id))) => {
                // The copy inherits from the `use` element, like a child.
                let mut context = own.clone();
                context.paint = self.paint_of(&own);
                for target in self.targets(id) {
                    if self.active[target.id().get_usize()] {
                        return Err(cycle(self.document, node));
                    }
                    self.visit(target, &context, Reached::Reference, depth + 1)?;
                }
            }
            ("use", Some(Href::Other)) => {
                self.omissions.insert(Omission::OtherFiles);
            }
            ("image" | "feImage", Some(Href::Other)) => {
                self.omissions.insert(Omission::LinkedImages);
            }
            (
                "feImage" | "linearGradient" | "radialGradient" | "pattern" | "filter",
                Some(Href::Local(id)),
            ) => {
                for target in self.targets(id) {
                    if self.active[target.id().get_usize()] {
                        return Err(cycle(self.document, node));
                    }
                    let context = self.ancestor_context(target);
                    self.visit(target, &context, Reached::Reference, depth + 1)?;
                }
            }
            ("textPath" | "tref", Some(Href::Local(id))) => {
                // Text follows the path, or copies the text, of the target element.
                for target in self.targets(id) {
                    let bytes = data_bytes(target, known(target).unwrap_or_default());
                    self.charge(0, bytes)?;
                }
            }
            _ => {}
        }
        for child in node.children().filter(Node::is_element) {
            if child.tag_name().name() == "foreignObject" && !switch_fallback(child) {
                self.omissions.insert(Omission::HtmlContent);
            }
            self.visit(child, &own, Reached::Structure, depth + 1)?;
        }
        self.active[index] = false;
        Ok(())
    }

    /// Instantiate the elements with `id` that the property accepts, `count` times.
    /// Content of a definition inherits from the definition's own ancestors, not from the
    /// referring element; `paint` is the referring element's, for `context-fill`.
    fn reference(
        &mut self,
        from: Node<'a, 'input>,
        id: &'a str,
        usage: Use,
        depth: usize,
        count: u64,
        paint: &Rc<[&'a str]>,
    ) -> io::Result<()> {
        for target in self.targets(id) {
            let accepted = matches!(
                (usage, known(target)),
                (
                    Use::Paint,
                    Some("pattern" | "linearGradient" | "radialGradient")
                ) | (Use::Marker, Some("marker"))
                    | (Use::Effect(Property::ClipPath), Some("clipPath"))
                    | (Use::Effect(Property::Mask), Some("mask"))
                    | (Use::Effect(Property::Filter), Some("filter"))
            );
            if !accepted {
                continue;
            }
            if self.active[target.id().get_usize()] {
                // usvg skips a marker within its own content; any other cycle is refused.
                if usage == Use::Marker {
                    continue;
                }
                return Err(cycle(self.document, from));
            }
            let mut context = self.ancestor_context(target);
            context.paint = paint.clone();
            self.repeat(count, |walk| {
                walk.visit(target, &context, Reached::Reference, depth + 1)
            })?;
        }
        Ok(())
    }

    /// Run `visit` once and charge its cost `count - 1` more times.
    fn repeat(
        &mut self,
        count: u64,
        visit: impl FnOnce(&mut Self) -> io::Result<()>,
    ) -> io::Result<()> {
        if count == 0 {
            return Ok(());
        }
        let (instances, data) = (self.instances, self.data);
        visit(self)?;
        let more = count - 1;
        self.charge(
            (self.instances - instances).saturating_mul(more),
            (self.data - data).saturating_mul(more),
        )
    }

    fn charge(&mut self, instances: u64, data: u64) -> io::Result<()> {
        self.instances = self.instances.saturating_add(instances);
        self.data = self.data.saturating_add(data);
        if tree_bytes(self.instances, self.data) > MAX_TREE_BYTES {
            return Err(limit(format!(
                "This SVG needs more than {} MiB for its elements and path data once its \
                 references are expanded, the limit for SVG images.",
                MAX_TREE_BYTES >> 20
            )));
        }
        Ok(())
    }

    fn targets(&self, id: &str) -> Vec<Node<'a, 'input>> {
        self.ids.get(id).cloned().unwrap_or_default()
    }

    /// The paint of an element, for `context-fill` and `context-stroke` in its marker or
    /// `use` content.
    fn paint_of(&self, own: &Context<'a>) -> Rc<[&'a str]> {
        let fill = &own.values[Property::Fill as usize];
        let stroke = &own.values[Property::Stroke as usize];
        if stroke.is_empty() {
            return fill.clone();
        }
        fill.iter().chain(stroke.iter()).copied().collect()
    }

    /// The values `node` declares, combined with the parent's.
    fn context(&mut self, node: Node<'a, 'input>, parent: &Context<'a>) -> Context<'a> {
        let declared = self.declared(node);
        let mut own = parent.clone();
        for (index, value) in own.values.iter_mut().enumerate() {
            if !declared.declared[index] {
                if index >= INHERITED {
                    *value = self.empty.clone();
                }
                continue;
            }
            let mut values = declared.targets[index].clone();
            if declared.inherit[index] {
                values.extend(parent.values[index].iter());
            }
            if declared.context[index] {
                values.extend(parent.paint.iter());
            }
            *value = Rc::from(values);
        }
        own
    }

    /// The context that a definition's content inherits: its document ancestors' values.
    fn ancestor_context(&mut self, node: Node<'a, 'input>) -> Context<'a> {
        let Some(parent) = node.parent_element() else {
            return self.blank();
        };
        let index = parent.id().get_usize();
        if let Some(context) = self.ancestors.get(&index) {
            return context.clone();
        }
        let above = self.ancestor_context(parent);
        let context = self.context(parent, &above);
        self.ancestors.insert(index, context.clone());
        context
    }

    fn declared(&mut self, node: Node<'a, 'input>) -> Rc<Declared<'a>> {
        let index = node.id().get_usize();
        if let Some(declared) = &self.declared[index] {
            return declared.clone();
        }
        let mut declared = Declared::default();
        let mut any = false;
        for attribute in node.attributes() {
            if !attribute
                .namespace()
                .is_none_or(|namespace| namespace == SVG_NAMESPACE)
            {
                continue;
            }
            if attribute.name() == "style" {
                for declaration in simplecss::DeclarationTokenizer::from(attribute.value()) {
                    any |= declared.add(declaration.name, declaration.value, &mut self.omissions);
                }
            } else {
                any |= declared.add(attribute.name(), attribute.value(), &mut self.omissions);
            }
        }
        for (selector, declarations) in &self.rules {
            if selector.matches(&XmlNode(node)) {
                for (name, value) in declarations {
                    any |= declared.add(name, value, &mut self.omissions);
                }
            }
        }
        let declared = if any {
            Rc::new(declared)
        } else {
            self.nothing.clone()
        };
        self.declared[index] = Some(declared.clone());
        declared
    }
}

/// The element's name when usvg converts it: SVG namespace (or none) and a known name.
fn known<'a>(node: Node<'a, '_>) -> Option<&'a str> {
    if !node
        .tag_name()
        .namespace()
        .is_none_or(|namespace| namespace == SVG_NAMESPACE)
    {
        return None;
    }
    let name = node.tag_name().name();
    matches!(
        name,
        "a" | "circle"
            | "clipPath"
            | "defs"
            | "ellipse"
            | "feBlend"
            | "feColorMatrix"
            | "feComponentTransfer"
            | "feComposite"
            | "feConvolveMatrix"
            | "feDiffuseLighting"
            | "feDisplacementMap"
            | "feDistantLight"
            | "feDropShadow"
            | "feFlood"
            | "feFuncA"
            | "feFuncB"
            | "feFuncG"
            | "feFuncR"
            | "feGaussianBlur"
            | "feImage"
            | "feMerge"
            | "feMergeNode"
            | "feMorphology"
            | "feOffset"
            | "fePointLight"
            | "feSpecularLighting"
            | "feSpotLight"
            | "feTile"
            | "feTurbulence"
            | "filter"
            | "g"
            | "image"
            | "line"
            | "linearGradient"
            | "marker"
            | "mask"
            | "path"
            | "pattern"
            | "polygon"
            | "polyline"
            | "radialGradient"
            | "rect"
            | "stop"
            | "style"
            | "svg"
            | "switch"
            | "symbol"
            | "text"
            | "textPath"
            | "tref"
            | "tspan"
            | "use"
    )
    .then_some(name)
}

/// Elements drawn only through a reference, never where they stand.
fn definition(name: &str) -> bool {
    matches!(
        name,
        "clipPath"
            | "defs"
            | "filter"
            | "linearGradient"
            | "marker"
            | "mask"
            | "pattern"
            | "radialGradient"
            | "style"
            | "symbol"
    )
}

fn shape(name: &str) -> bool {
    matches!(
        name,
        "path" | "rect" | "circle" | "ellipse" | "line" | "polyline" | "polygon"
    )
}

/// Elements whose text usvg lays out; a link inside text paints like a `tspan`.
fn text_content(name: &str) -> bool {
    matches!(name, "text" | "tspan" | "textPath" | "tref" | "a")
}

/// A bound on a shape's vertices, where a mid marker is drawn. In path data every vertex
/// needs two numbers or a closing `z`, except horizontal and vertical lines (one number)
/// and arcs (at least five numbers as written, for up to four curves).
fn vertices(node: Node<'_, '_>, name: &str) -> u64 {
    match name {
        "path" => {
            let data = node.attribute("d").unwrap_or_default();
            let closes = data
                .bytes()
                .filter(|byte| matches!(byte, b'z' | b'Z'))
                .count() as u64;
            let numbers = numbers(data);
            let per_vertex = if data.contains(['H', 'h', 'V', 'v', 'A', 'a']) {
                numbers
            } else {
                numbers.div_ceil(2)
            };
            per_vertex + closes + 1
        }
        "polyline" | "polygon" => {
            numbers(node.attribute("points").unwrap_or_default()).div_ceil(2) + 2
        }
        "line" => 2,
        // Rectangles with rounded corners, circles and ellipses become a few curves.
        _ => 16,
    }
}

/// Numbers in a number list or path data: an optional sign, digits with at most one
/// decimal point and an optional exponent. Arc flags written together with the next
/// number count once with it, which still leaves at least five numbers per arc.
fn numbers(text: &str) -> u64 {
    let bytes = text.as_bytes();
    let (mut count, mut index) = (0, 0);
    while index < bytes.len() {
        let start = index;
        if matches!(bytes[index], b'+' | b'-') {
            index += 1;
        }
        let (mut digits, mut point) = (false, false);
        while let Some(&byte) = bytes.get(index) {
            match byte {
                b'0'..=b'9' => digits = true,
                b'.' if !point => point = true,
                _ => break,
            }
            index += 1;
        }
        if digits {
            count += 1;
            if bytes
                .get(index)
                .is_some_and(|byte| matches!(byte, b'e' | b'E'))
            {
                let mut exponent = index + 1;
                if bytes
                    .get(exponent)
                    .is_some_and(|byte| matches!(byte, b'+' | b'-'))
                {
                    exponent += 1;
                }
                if bytes.get(exponent).is_some_and(u8::is_ascii_digit) {
                    index = exponent;
                    while bytes.get(index).is_some_and(u8::is_ascii_digit) {
                        index += 1;
                    }
                }
            }
        } else if index == start {
            index += 1;
        }
    }
    count
}

/// Bytes the converter parses for one instance: attribute values and direct text.
fn data_bytes(node: Node<'_, '_>, name: &str) -> u64 {
    let attributes: usize = node
        .attributes()
        .map(|attribute| attribute.value().len())
        .sum();
    let text: usize = if text_content(name) {
        node.children()
            .filter(Node::is_text)
            .filter_map(|child| child.text())
            .map(str::len)
            .sum()
    } else {
        0
    };
    (attributes + text) as u64
}

enum Href<'a> {
    Local(&'a str),
    Other,
}

/// The reference usvg follows: an unprefixed `href` wins over `xlink:href` in any
/// attribute order; other namespaces are ignored. `data:` images are embedded, not links.
fn href<'a>(node: Node<'a, '_>) -> Option<Href<'a>> {
    const XLINK: &str = "http://www.w3.org/1999/xlink";
    let value = node
        .attributes()
        .find(|attribute| attribute.name() == "href" && attribute.namespace().is_none())
        .or_else(|| {
            node.attributes().find(|attribute| {
                attribute.name() == "href" && attribute.namespace() == Some(XLINK)
            })
        })?
        .value()
        .trim();
    if value.is_empty()
        || value
            .get(..5)
            .is_some_and(|scheme| scheme.eq_ignore_ascii_case("data:"))
    {
        return None;
    }
    Some(match value.strip_prefix('#') {
        Some(id) => Href::Local(id),
        None => Href::Other,
    })
}

/// A `foreignObject` inside a `switch` with another drawable child shows that child.
fn switch_fallback(node: Node<'_, '_>) -> bool {
    node.parent_element().is_some_and(|parent| {
        known(parent) == Some("switch")
            && parent
                .children()
                .filter(Node::is_element)
                .any(|sibling| sibling != node && known(sibling).is_some())
    })
}

fn line(document: &Document<'_>, node: Node<'_, '_>) -> u32 {
    document.text_pos_at(node.range().start).row
}

fn depth_limit(document: &Document<'_>, node: Node<'_, '_>) -> io::Error {
    limit(format!(
        "This SVG nests elements and references more than {MAX_DEPTH} levels deep (line {}), \
         the limit for SVG images.",
        line(document, node)
    ))
}

fn cycle(document: &Document<'_>, node: Node<'_, '_>) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!(
            "This SVG is invalid: the reference on line {} leads back to itself.",
            line(document, node)
        ),
    )
}

/// Style-rule matching exactly as usvg does it, so the same rules apply.
struct XmlNode<'a, 'input>(Node<'a, 'input>);

impl simplecss::Element for XmlNode<'_, '_> {
    fn parent_element(&self) -> Option<Self> {
        self.0.parent_element().map(XmlNode)
    }

    fn prev_sibling_element(&self) -> Option<Self> {
        self.0.prev_sibling_element().map(XmlNode)
    }

    fn has_local_name(&self, name: &str) -> bool {
        self.0.tag_name().name() == name
    }

    fn attribute_matches(&self, name: &str, operator: simplecss::AttributeOperator) -> bool {
        self.0
            .attribute(name)
            .is_some_and(|value| operator.matches(value))
    }

    fn pseudo_class_matches(&self, class: simplecss::PseudoClass) -> bool {
        matches!(class, simplecss::PseudoClass::FirstChild)
            && self.0.prev_sibling_element().is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::{check, numbers};

    fn instances(body: &str) -> u64 {
        let mut text = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">{body}</svg>"#
        );
        let document = super::super::document::parse(&mut text).unwrap();
        check(&document).unwrap().instances
    }

    #[test]
    fn markers_count_per_vertex_with_the_paint_and_rules_they_take() {
        const DEFS: &str = r##"<pattern id="p" width="1" height="1"><rect width="1" height="1"/><rect width="1" height="1"/><rect width="1" height="1"/><rect width="1" height="1"/></pattern><marker id="m"><path d="M0 0h1" fill="context-fill"/></marker>"##;
        let plain = instances(&format!(
            r##"{DEFS}<path d="M0 0L1 0L2 0L3 0L4 0" fill="url(#p)"/>"##
        ));
        let marked = instances(&format!(
            r##"{DEFS}<path d="M0 0L1 0L2 0L3 0L4 0" fill="url(#p)" marker-mid="url(#m)"/>"##
        ));
        // At least the three middle vertices draw the marker, and each marker path paints
        // with the pattern of the shape (context-fill): five elements per vertex.
        assert!(marked >= plain + 3 * 7, "{plain} {marked}");
        let styled = instances(&format!(
            r##"<style>.arrow {{ marker-mid: url(#m) }}</style>{DEFS}<path class="arrow" d="M0 0L1 0L2 0L3 0L4 0" fill="url(#p)"/>"##
        ));
        assert!(styled >= marked, "{marked} {styled}");
    }

    #[test]
    fn numbers_are_counted_like_an_svg_number_parser() {
        for (text, count) in [
            ("", 0),
            ("M0 0L10-5", 4),
            ("0.5.5", 2),
            ("-.5e-3+2E2", 2),
            ("1e", 1),
            ("--5", 1),
            ("12.25,600.0 13,4", 4),
            ("a1 1 0 0110 10", 5),
        ] {
            assert_eq!(numbers(text), count, "{text}");
        }
    }
}
