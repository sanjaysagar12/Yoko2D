use std::collections::HashMap; // accumulates one element's attributes, and backs Document::from_parts's (always-empty) base_variables here

use quick_xml::events::{BytesDecl, BytesStart, BytesText, Event}; // the specific event/payload types used while reading and writing
use quick_xml::Writer; // the XML writer this module wraps
use thiserror::Error; // brings in the Error derive macro used below

use core_lib::{Document, ObjectId, ToolKind, ToolRecord}; // the Document types this module serializes to/from XML

/// Everything that can go wrong saving or loading a pattern file.
#[derive(Debug, Error)] // Debug: printable in test failures; Error: implements std::error::Error via thiserror
pub enum PatternFileError {
    /// Reading/writing the underlying bytes failed.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error), // the underlying I/O failure

    /// The XML itself is malformed (unclosed tags, invalid syntax, etc.).
    #[error("XML error: {0}")]
    Xml(#[from] quick_xml::Error), // the underlying XML parse/write failure

    /// The parsed history couldn't be turned into a valid `Document` (e.g.
    /// two records claiming the same id).
    #[error("invalid document: {0}")]
    Document(#[from] core_lib::DocumentError), // the underlying Document-construction failure

    /// A `<point type="...">` named a type this parser version doesn't
    /// recognize.
    #[error("unknown tool type {0:?}")]
    UnknownToolType(String), // the unrecognized type value

    /// A required attribute was absent from an element.
    #[error("<{tag}> is missing required attribute {attribute:?}")]
    MissingAttribute {
        tag: String,       // the element that was missing an attribute
        attribute: String, // the attribute name that was expected but absent
    },

    /// An attribute was present but couldn't be parsed as its expected type.
    #[error("<{tag}> attribute {attribute:?} has invalid value {value:?}")]
    InvalidAttributeValue {
        tag: String,       // the element the bad attribute was on
        attribute: String, // which attribute failed to parse
        value: String,     // the raw text that failed to parse
    },
}

/// Serializes `doc` (and, optionally, a reference to where its measurements
/// live) into a pattern-file XML string.
///
/// Mirrors the real Seamly2D `.val` format's attribute-heavy, tagged-union
/// shape (`type="..."` selects which other attributes are meaningful) by
/// hand-rolling the XML via `quick_xml`'s writer API, the same way the
/// original C++ app writes its files via manual `SetAttribute` calls,
/// rather than deriving it through serde.
pub fn serialize_document(
    doc: &Document, // the tool history + structure to persist (base_variables is deliberately NOT persisted; see the <history> loop below)
    measurements_path: Option<&str>, // an optional reference to the measurement file this pattern expects, stored as a path only
) -> Result<String, PatternFileError> {
    write_document(doc, measurements_path).map_err(PatternFileError::from) // do the actual writing under plain io::Result (what quick_xml's writer API expects), then lift the one possible error kind at the end
}

/// Does the actual XML writing, returning a plain `std::io::Result` because
/// that's what `quick_xml::Writer`'s `write_inner_content` closures in this
/// version require; `serialize_document` above is what turns that into a
/// `PatternFileError` for callers.
fn write_document(doc: &Document, measurements_path: Option<&str>) -> std::io::Result<String> {
    let buffer: Vec<u8> = Vec::new(); // in-memory byte buffer the writer writes into; no real file touched during serialization itself
                                      // Pretty-printing (2-space indent) is purely for human readability of a
                                      // saved file opened in a text editor — it has no semantic effect on
                                      // parsing, since deserialize_document doesn't care about whitespace
                                      // between elements.
    let mut writer = Writer::new_with_indent(buffer, b' ', 2); // indent_char=' ', indent_size=2 spaces per nesting level

    writer.write_event(Event::Decl(BytesDecl::new("1.0", Some("UTF-8"), None)))?; // <?xml version="1.0" encoding="UTF-8"?>

    writer
        .create_element("pattern")
        .write_inner_content(|writer| {
            writer
                .create_element("version") // exists for future format-migration support, mirroring Seamly2D's own <version> tag and converter classes — not implemented yet, so this is a fixed literal for now
                .write_text_content(BytesText::new("0.1"))?; // a fixed literal for now

            writer
                .create_element("unit") // hardcoded for now — unit selection/conversion is out of scope for this phase, the same limitation flagged in Phase 5's measurement loading
                .write_text_content(BytesText::new("cm"))?;

            if let Some(path) = measurements_path {
                // Stores only a PATH REFERENCE, not measurement values
                // themselves — mirrors how the real .val format's
                // <measurements> tag works, and keeps measurement
                // loading/reloading entirely owned by Phase 5/6's separate
                // pipeline rather than duplicated into this file format.
                writer
                    .create_element("measurementsPath")
                    .write_text_content(BytesText::new(path))?;
            }

            writer
                .create_element("history")
                .write_inner_content(|writer| {
                    // Iterated in document order, deliberately: tool order
                    // determines dependency resolution order in
                    // core_lib::recompute_all, so the saved file must preserve the
                    // exact sequence, not merely the set of tools it contains.
                    for record in doc.history() {
                        write_tool_record(writer, record)?; // one element per tool, dispatching on its kind
                    }
                    Ok(())
                })?;

            Ok(())
        })?;

    let bytes = writer.into_inner(); // unwrap the Writer to reclaim the underlying Vec<u8> buffer
                                     // Every piece of text written above came from a Rust &str/String, which
                                     // is always valid UTF-8 by the type system, so this conversion cannot
                                     // fail in practice — handled without unwrap/panic anyway, per this
                                     // crate's policy of never panicking on data-shaped input.
    String::from_utf8(bytes)
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))
}

/// Writes the single XML element corresponding to one [`ToolRecord`],
/// dispatching on its [`ToolKind`].
fn write_tool_record(writer: &mut Writer<Vec<u8>>, record: &ToolRecord) -> std::io::Result<()> {
    match &record.kind {
        // dispatch on which kind of tool this record is
        ToolKind::BasePoint { name, x, y } => {
            writer
                .create_element("point")
                .with_attribute(("type", "basePoint")) // the type attribute selects which other attributes are meaningful — a tagged union encoded as XML attributes, same as the real .val format
                .with_attribute(("id", record.id.raw().to_string().as_str())) // this tool's own id
                .with_attribute(("name", name.as_str())) // the point's user-facing label
                .with_attribute(("x", x.to_string().as_str())) // literal x coordinate
                .with_attribute(("y", y.to_string().as_str())) // literal y coordinate
                .write_empty()?; // a self-closing <point .../>: a BasePoint has no child content
        }
        ToolKind::EndLine {
            name,
            base_point,
            angle_formula,
            length_formula,
        } => {
            writer
                .create_element("point")
                .with_attribute(("type", "endLine")) // selects the endLine interpretation of <point>
                .with_attribute(("id", record.id.raw().to_string().as_str())) // this tool's own id
                .with_attribute(("name", name.as_str())) // the point's user-facing label
                .with_attribute(("basePoint", base_point.raw().to_string().as_str())) // which earlier tool's point this one is measured from
                .with_attribute(("angle", angle_formula.as_str())) // the FORMULA STRING, not a resolved number — same "store formulas not values" principle as Phase 3/4
                .with_attribute(("length", length_formula.as_str())) // likewise, the length formula string, unevaluated
                .write_empty()?;
        }
        ToolKind::Line { p1, p2 } => {
            writer
                .create_element("line")
                .with_attribute(("id", record.id.raw().to_string().as_str())) // this tool's own id
                .with_attribute(("firstPoint", p1.raw().to_string().as_str())) // the line's first endpoint, by id
                .with_attribute(("secondPoint", p2.raw().to_string().as_str())) // the line's second endpoint, by id
                .write_empty()?;
        }
    }
    Ok(()) // this record's element was written successfully
}

/// Parses `xml` back into a [`Document`] plus whatever `<measurementsPath>`
/// text (if any) it contained.
///
/// Base variables are never populated here — see the comment above the
/// `Document::from_parts` call at the bottom of this function for why.
pub fn deserialize_document(xml: &str) -> Result<(Document, Option<String>), PatternFileError> {
    let mut reader = quick_xml::Reader::from_str(xml); // a reader over the in-memory XML text, borrowing directly from `xml` (no intermediate buffer needed)

    let mut history: Vec<ToolRecord> = Vec::new(); // accumulates tool records as <point>/<line> elements are encountered, in document order
    let mut measurements_path: Option<String> = None; // set once (if ever) a <measurementsPath> element's text content is read
    let mut awaiting_measurements_path_text = false; // true right after a <measurementsPath> start tag, until its Text event (if any) is consumed

    loop {
        // pull XML events one at a time until Eof or a parse error
        match reader.read_event()? {
            // any malformed-XML failure here propagates as PatternFileError::Xml via #[from]
            Event::Eof => break, // reached the end of the document: stop reading
            Event::Start(start) | Event::Empty(start) => {
                // Start and Empty (self-closing) tags carry the same attribute data; this format only ever
                // uses Empty for <point>/<line>, but both are accepted here in case a file writes them otherwise
                awaiting_measurements_path_text = false; // reset any pending capture from a previous, textless <measurementsPath/> before starting a new element
                let tag = String::from_utf8_lossy(start.name().as_ref()).into_owned(); // this element's tag name, for error messages and dispatch below
                match tag.as_str() {
                    "point" => {
                        let record = parse_point(&start)?; // parse type/id/... attributes into a ToolRecord, or a typed error
                        history.push(record); // record it in document order
                    }
                    "line" => {
                        let record = parse_line(&start)?; // parse id/firstPoint/secondPoint attributes into a ToolRecord
                        history.push(record); // record it in document order
                    }
                    "measurementsPath" => {
                        awaiting_measurements_path_text = true; // the very next Text event (if any) is this element's path value
                    }
                    _ => {
                        // Any other element (<pattern>, <history>, <version>,
                        // <unit>, and any future top-level element this
                        // parser version doesn't know about) is silently
                        // skipped: this lets the format gain new elements in
                        // later phases without breaking older parser code,
                        // as long as those new elements aren't load-bearing
                        // for what this phase reads.
                    }
                }
            }
            Event::Text(text) if awaiting_measurements_path_text => {
                // this Text event belongs to the <measurementsPath> element opened just above
                let decoded = text.decode().map_err(quick_xml::Error::from)?; // decode the raw bytes as UTF-8 text
                let unescaped =
                    quick_xml::escape::unescape(&decoded).map_err(quick_xml::Error::from)?; // resolve XML entity escapes (e.g. &amp;) written by the serializer's BytesText::new
                measurements_path = Some(unescaped.into_owned()); // capture the fully decoded path text
                awaiting_measurements_path_text = false; // consumed; don't let a later, unrelated Text event be mistaken for this one
            }
            _ => {
                // any other event (an unrelated Text node, comments, CDATA, processing instructions, XML declaration, etc.): not load-bearing for this format, ignored
            }
        }
    }

    // base_variables is intentionally empty here: measurements are loaded
    // separately via Phase 5/6's pipeline (parse_measurements_str /
    // load_measurements_from_file + Document::apply_measurements), never
    // embedded directly in this file format — this file only ever stores a
    // *reference* to where they live (measurements_path above).
    let document = Document::from_parts(history, HashMap::new())?; // validates no two records share an id, and restores the id counter; propagates DocumentError via #[from]
    Ok((document, measurements_path)) // hand back both the restored Document and whatever measurements path (if any) the file referenced
}

/// Parses a `<point .../>` element's attributes into a [`ToolRecord`],
/// dispatching on its `type` attribute — the tagged-union encoding this
/// format uses for the different point-producing tools.
fn parse_point(start: &BytesStart) -> Result<ToolRecord, PatternFileError> {
    let attrs = read_attributes(start)?; // collect every attribute on this element into a name -> value map
    let type_value = require_attr("point", &attrs, "type")?; // which point variant this element encodes; missing -> MissingAttribute

    match type_value {
        "basePoint" => {
            let id = parse_id("point", &attrs, "id")?; // this tool's own id
            let name = require_attr("point", &attrs, "name")?.to_string(); // the point's user-facing label
            let x = parse_f64("point", &attrs, "x")?; // literal x coordinate
            let y = parse_f64("point", &attrs, "y")?; // literal y coordinate
            Ok(ToolRecord {
                id,                                       // the id parsed above
                kind: ToolKind::BasePoint { name, x, y }, // no formulas: a literal starting point
            })
        }
        "endLine" => {
            let id = parse_id("point", &attrs, "id")?; // this tool's own id
            let name = require_attr("point", &attrs, "name")?.to_string(); // the point's user-facing label
            let base_point = parse_id("point", &attrs, "basePoint")?; // which earlier tool's point this one is measured from
                                                                      // angle/length are read as plain strings, NOT parsed as numbers: they are formula strings, re-evaluated on every recompute (Phase 2/3), not resolved values
            let angle_formula = require_attr("point", &attrs, "angle")?.to_string(); // the angle formula, unevaluated
            let length_formula = require_attr("point", &attrs, "length")?.to_string(); // the length formula, unevaluated
            Ok(ToolRecord {
                id, // the id parsed above
                kind: ToolKind::EndLine {
                    name,
                    base_point,
                    angle_formula,
                    length_formula,
                },
            })
        }
        other => Err(PatternFileError::UnknownToolType(other.to_string())), // future-proofs against files referencing tool types this parser version doesn't know about, rather than silently ignoring or misinterpreting them
    }
}

/// Parses a `<line .../>` element's attributes into a [`ToolRecord`].
fn parse_line(start: &BytesStart) -> Result<ToolRecord, PatternFileError> {
    let attrs = read_attributes(start)?; // collect every attribute on this element into a name -> value map
    let id = parse_id("line", &attrs, "id")?; // this tool's own id
    let p1 = parse_id("line", &attrs, "firstPoint")?; // the line's first endpoint, by id
    let p2 = parse_id("line", &attrs, "secondPoint")?; // the line's second endpoint, by id
    Ok(ToolRecord {
        id,                              // the id parsed above
        kind: ToolKind::Line { p1, p2 }, // a straight line between two existing points, referenced by id
    })
}

/// Collects every attribute on `start` into a plain name -> value map, with
/// XML entity-escaping already resolved, so callers can look attributes up
/// by name without re-walking the raw attribute list for each one.
fn read_attributes(start: &BytesStart) -> Result<HashMap<String, String>, PatternFileError> {
    let mut attrs = HashMap::new(); // accumulates name -> value pairs for this element
    for attr_result in start.attributes() {
        // walk every raw attribute on this start/empty tag
        let attr = attr_result.map_err(quick_xml::Error::from)?; // convert AttrError into quick_xml::Error, which #[from] then lifts into PatternFileError
        let key = String::from_utf8_lossy(attr.key.as_ref()).into_owned(); // attribute name, decoded as UTF-8
        let value = attr
            .normalized_value(quick_xml::XmlVersion::Explicit1_0)? // attribute value, with XML entity-escaping (e.g. &amp;) resolved; version matches the "1.0" we write in the XML declaration
            .into_owned();
        attrs.insert(key, value); // record this attribute
    }
    Ok(attrs) // every attribute on this element, ready for lookup by name
}

/// Looks up a required attribute by name, or reports exactly which one is missing.
fn require_attr<'a>(
    tag: &str, // the element this attribute is expected on, for the error message
    attrs: &'a HashMap<String, String>, // the element's already-collected attributes
    name: &str, // the attribute name being looked up
) -> Result<&'a str, PatternFileError> {
    attrs
        .get(name) // Option<&String>
        .map(|value| value.as_str()) // Option<&str>
        .ok_or_else(|| PatternFileError::MissingAttribute {
            tag: tag.to_string(),        // which element was missing the attribute
            attribute: name.to_string(), // which attribute was expected but absent
        })
}

/// Parses a required attribute as an [`ObjectId`] (a plain non-negative
/// integer in the file, exactly as [`ObjectId::raw`] wrote it out).
fn parse_id(
    tag: &str,
    attrs: &HashMap<String, String>,
    name: &str,
) -> Result<ObjectId, PatternFileError> {
    let value = require_attr(tag, attrs, name)?; // the raw attribute text, or a MissingAttribute error
    let raw: u32 = value
        .parse()
        .map_err(|_| PatternFileError::InvalidAttributeValue {
            tag: tag.to_string(),        // which element the bad attribute was on
            attribute: name.to_string(), // which attribute failed to parse
            value: value.to_string(),    // the raw text that failed to parse
        })?; // reject anything that isn't a valid u32, rather than panicking
    Ok(ObjectId::from_raw(raw)) // reconstruct the id exactly as saved; see ObjectId::from_raw's own doc comment for why this is safe here
}

/// Parses a required attribute as an `f64` (a literal coordinate in the file).
fn parse_f64(
    tag: &str,
    attrs: &HashMap<String, String>,
    name: &str,
) -> Result<f64, PatternFileError> {
    let value = require_attr(tag, attrs, name)?; // the raw attribute text, or a MissingAttribute error
    value
        .parse()
        .map_err(|_| PatternFileError::InvalidAttributeValue {
            tag: tag.to_string(),        // which element the bad attribute was on
            attribute: name.to_string(), // which attribute failed to parse
            value: value.to_string(),    // the raw text that failed to parse
        }) // reject anything that isn't a valid f64, rather than panicking
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_lib::{recompute_all, DocumentError, ObjectId as CoreObjectId, Variable};

    fn sample_document() -> (Document, CoreObjectId, CoreObjectId, CoreObjectId) {
        let mut doc = Document::default();
        let a = doc.add_base_point("A", 0.0, 0.0);
        let a1 = doc.add_end_line("A1", a, "0", "height_scapula/10").unwrap();
        let line = doc.add_line(a, a1).unwrap();
        (doc, a, a1, line)
    }

    #[test]
    fn serialize_contains_expected_elements_and_attributes() {
        let (doc, ..) = sample_document();
        let xml = serialize_document(&doc, None).unwrap();

        assert!(xml.contains(r#"type="basePoint""#));
        assert!(xml.contains(r#"type="endLine""#));
        assert!(xml.contains(r#"length="height_scapula/10""#));
        assert!(xml.contains(r#"angle="0""#));
        assert!(xml.contains("<line "));
    }

    #[test]
    fn round_trip_preserves_recompute_result() {
        let (mut original, ..) = sample_document();
        original.set_variable("height_scapula", Variable::Measurement { value: 40.0 });

        let xml = serialize_document(&original, None).unwrap();
        let (mut restored, _path) = deserialize_document(&xml).unwrap();
        // base_variables are intentionally NOT persisted; re-seed identically so recompute is comparable.
        restored.set_variable("height_scapula", Variable::Measurement { value: 40.0 });

        let original_data = recompute_all(&original).unwrap();
        let restored_data = recompute_all(&restored).unwrap();
        assert_eq!(original_data, restored_data);
    }

    #[test]
    fn round_trip_preserves_tool_ids_and_kinds() {
        let (original, _a, a1, _line) = sample_document();
        let xml = serialize_document(&original, None).unwrap();
        let (restored, _path) = deserialize_document(&xml).unwrap();

        let original_tool = original.get_tool(a1).unwrap();
        let restored_tool = restored.get_tool(a1).unwrap();
        assert_eq!(original_tool, restored_tool);
        assert!(matches!(restored_tool.kind, ToolKind::EndLine { .. }));
    }

    #[test]
    fn deserializing_restores_next_id_past_every_loaded_id() {
        let (original, ..) = sample_document();
        let xml = serialize_document(&original, None).unwrap();
        let (mut restored, _path) = deserialize_document(&xml).unwrap();

        // Capture every id present BEFORE mutating, since add_base_point below appends its own new record to history.
        let loaded_ids: Vec<CoreObjectId> = restored.history().iter().map(|r| r.id).collect();
        let new_id = restored.add_base_point("Z", 9.0, 9.0);
        for id in loaded_ids {
            assert!(new_id > id); // the freshly allocated id exceeds every id that was loaded from the file
        }
    }

    #[test]
    fn malformed_xml_is_reported_not_panicked() {
        let result = deserialize_document("<pattern><history><point");
        assert!(result.is_err());
    }

    #[test]
    fn duplicate_point_ids_are_reported() {
        let xml = r#"<pattern><history>
            <point type="basePoint" id="1" name="A" x="0" y="0"/>
            <point type="basePoint" id="1" name="B" x="1" y="1"/>
        </history></pattern>"#;

        let err = deserialize_document(xml).unwrap_err();
        assert!(matches!(
            err,
            PatternFileError::Document(DocumentError::DuplicateId(_))
        ));
    }

    #[test]
    fn unknown_tool_type_is_reported() {
        let xml = r#"<pattern><history><point type="bogusTool" id="1"/></history></pattern>"#;

        let err = deserialize_document(xml).unwrap_err();
        match err {
            PatternFileError::UnknownToolType(t) => assert_eq!(t, "bogusTool"),
            other => panic!("expected UnknownToolType, got {other:?}"),
        }
    }

    #[test]
    fn missing_attribute_is_reported() {
        let xml = r#"<pattern><history><point type="basePoint" id="1" name="A" x="0"/></history></pattern>"#;

        let err = deserialize_document(xml).unwrap_err();
        match err {
            PatternFileError::MissingAttribute { tag, attribute } => {
                assert_eq!(tag, "point");
                assert_eq!(attribute, "y");
            }
            other => panic!("expected MissingAttribute, got {other:?}"),
        }
    }

    #[test]
    fn invalid_attribute_value_is_reported() {
        let xml = r#"<pattern><history><point type="basePoint" id="1" name="A" x="not_a_number" y="0"/></history></pattern>"#;

        let err = deserialize_document(xml).unwrap_err();
        match err {
            PatternFileError::InvalidAttributeValue {
                tag,
                attribute,
                value,
            } => {
                assert_eq!(tag, "point");
                assert_eq!(attribute, "x");
                assert_eq!(value, "not_a_number");
            }
            other => panic!("expected InvalidAttributeValue, got {other:?}"),
        }
    }

    #[test]
    fn loading_fixture_and_recomputing_matches_hand_verified_value() {
        // CARGO_MANIFEST_DIR is crates/io at compile time; the fixtures directory
        // lives at the workspace root, two levels up from there (same pattern as Phase 5's fixture test).
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/patterns/simplified_seamly2d_style.xml");
        let xml = std::fs::read_to_string(&path).unwrap();

        let (mut doc, measurements_path) = deserialize_document(&xml).unwrap();
        assert_eq!(measurements_path, Some("sample.json".to_string()));

        doc.set_variable("height_scapula", Variable::Measurement { value: 40.0 });
        let data = recompute_all(&doc).unwrap();

        // Hand-verified: A1 is at angle 0 degrees, length height_scapula/10 = 4.0, from A (0,0) -> (4.0, 0.0).
        let a1_id = CoreObjectId::from_raw(2); // matches the fixture's own id="2" for the endLine point
        let point = data.get_point(a1_id).unwrap();
        assert!((point.x - 4.0).abs() < 1e-9);
        assert!((point.y - 0.0).abs() < 1e-9);
    }

    #[test]
    fn serialize_with_measurements_path_round_trips() {
        let (doc, ..) = sample_document();
        let xml = serialize_document(&doc, Some("my_measurements.json")).unwrap();
        assert!(xml.contains("<measurementsPath>my_measurements.json</measurementsPath>"));

        let (_restored, path) = deserialize_document(&xml).unwrap();
        assert_eq!(path, Some("my_measurements.json".to_string()));
    }

    #[test]
    fn serialize_without_measurements_path_omits_the_element() {
        let (doc, ..) = sample_document();
        let xml = serialize_document(&doc, None).unwrap();
        assert!(!xml.contains("measurementsPath"));

        let (_restored, path) = deserialize_document(&xml).unwrap();
        assert_eq!(path, None);
    }

    #[test]
    fn deserialized_document_has_empty_base_variables() {
        let (mut doc, ..) = sample_document();
        doc.set_variable("height_scapula", Variable::Measurement { value: 40.0 }); // present in the ORIGINAL, pre-serialization document
        let xml = serialize_document(&doc, Some("whatever.json")).unwrap();

        let (restored, _path) = deserialize_document(&xml).unwrap();
        assert_eq!(restored.base_variables().count(), 0);
    }
}
