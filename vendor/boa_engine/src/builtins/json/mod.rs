//! Boa's implementation of ECMAScript's global `JSON` object.
//!
//! The `JSON` object contains methods for parsing [JavaScript Object Notation (JSON)][spec]
//! and converting values to JSON. It can't be called or constructed, and aside from its
//! two method properties, it has no interesting functionality of its own.
//!
//! More information:
//!  - [ECMAScript reference][spec]
//!  - [MDN documentation][mdn]
//!  - [JSON specification][json]
//!
//! [spec]: https://tc39.es/ecma262/#sec-json
//! [json]: https://www.json.org/json-en.html
//! [mdn]: https://developer.mozilla.org/en-US/docs/Web/JavaScript/Reference/Global_Objects/JSON

use std::{borrow::Cow, iter::once};

use rustc_hash::FxHashSet;

use boa_gc::{Finalize, Trace};
use boa_macros::utf16;
use itertools::Itertools;

use crate::JsExpect;
use crate::{
    Context, JsArgs, JsBigInt, JsData, JsResult, JsString, JsValue,
    builtins::BuiltInObject,
    context::intrinsics::Intrinsics,
    error::JsNativeError,
    js_string,
    object::{IntegrityLevel, JsObject, internal_methods::InternalMethodPropertyContext},
    property::{Attribute, PropertyNameKind},
    realm::Realm,
    string::{CodePoint, StaticJsStrings},
    symbol::JsSymbol,
    value::IntegerOrInfinity,
};

use super::{BuiltInBuilder, IntrinsicObject};

/// Converts a nibble (0-15) to its lowercase hex digit as a UTF-16 code unit.
const fn to_hex_digit(val: u16) -> u16 {
    match val & 0xF {
        v @ 0..=9 => b'0' as u16 + v,
        v => b'a' as u16 + (v - 10),
    }
}

#[cfg(test)]
mod tests;

/// Marker struct for the `[[IsRawJSON]]` internal slot.
#[derive(Debug, Trace, Finalize, JsData)]
pub(crate) struct RawJson;

trait JsonInput<'a>: Copy {
    fn len(self) -> usize;
    fn code_unit(self, idx: usize) -> u16;
    fn slice_to_js_string(self, start: usize, end: usize) -> JsString;
    fn ascii_bytes(self, start: usize, end: usize) -> Cow<'a, [u8]>;
}

impl<'a> JsonInput<'a> for &'a [u8] {
    #[inline]
    fn len(self) -> usize {
        <[u8]>::len(self)
    }

    #[inline]
    fn code_unit(self, idx: usize) -> u16 {
        u16::from(self[idx])
    }

    #[inline]
    fn slice_to_js_string(self, start: usize, end: usize) -> JsString {
        JsString::from(boa_string::JsStr::latin1(&self[start..end]))
    }

    #[inline]
    fn ascii_bytes(self, start: usize, end: usize) -> Cow<'a, [u8]> {
        Cow::Borrowed(&self[start..end])
    }
}

impl<'a> JsonInput<'a> for &'a [u16] {
    #[inline]
    fn len(self) -> usize {
        <[u16]>::len(self)
    }

    #[inline]
    fn code_unit(self, idx: usize) -> u16 {
        self[idx]
    }

    #[inline]
    fn slice_to_js_string(self, start: usize, end: usize) -> JsString {
        js_string!(&self[start..end])
    }

    #[inline]
    fn ascii_bytes(self, start: usize, end: usize) -> Cow<'a, [u8]> {
        Cow::Owned(self[start..end].iter().map(|&u| u as u8).collect())
    }
}

#[derive(Debug)]
enum JsonNode {
    /// A primitive value (string, number, boolean, null) with its original source text and parsed value.
    Primitive(JsString, JsValue),
    /// An array of child nodes.
    Array(Vec<JsonNode>),
    /// An object with key-value pairs.
    Object(Vec<(JsString, JsonNode)>),
}

/// JavaScript `JSON` global object.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct Json;

impl IntrinsicObject for Json {
    fn init(realm: &Realm) {
        let to_string_tag = JsSymbol::to_string_tag();
        let attribute = Attribute::READONLY | Attribute::NON_ENUMERABLE | Attribute::CONFIGURABLE;

        BuiltInBuilder::with_intrinsic::<Self>(realm)
            .static_method(Self::parse, js_string!("parse"), 2)
            .static_method(Self::stringify, js_string!("stringify"), 3)
            .static_method(Self::raw_json, js_string!("rawJSON"), 1)
            .static_method(Self::is_raw_json, js_string!("isRawJSON"), 1)
            .static_property(to_string_tag, Self::NAME, attribute)
            .build();
    }

    fn get(intrinsics: &Intrinsics) -> JsObject {
        intrinsics.objects().json()
    }
}

impl BuiltInObject for Json {
    const NAME: JsString = StaticJsStrings::JSON;
}

impl Json {
    /// `JSON.parse( text[, reviver] )`
    ///
    /// This `JSON` method parses a JSON string, constructing the JavaScript value or object described by the string.
    ///
    /// An optional `reviver` function can be provided to perform a transformation on the resulting object before it is returned.
    ///
    /// More information:
    ///  - [ECMAScript reference][spec]
    ///  - [MDN documentation][mdn]
    ///
    /// [spec]: https://tc39.es/ecma262/#sec-json.parse
    /// [mdn]: https://developer.mozilla.org/en-US/docs/Web/JavaScript/Reference/Global_Objects/JSON/parse
    pub(crate) fn parse(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
        // 1. Let jsonString be ? ToString(text).
        let js_str = args
            .first()
            .cloned()
            .unwrap_or_default()
            .to_string(context)?;

        // Check if a reviver is provided, to determine if we need source text tracking
        let reviver_obj = args.get_or_undefined(1).as_callable();

        // 2-10. Parse JSON text directly per ECMA-404 & ECMAScript 25.5.1.
        let (unfiltered, source_tree) = match js_str.as_str().variant() {
            boa_string::JsStrVariant::Latin1(bytes) => {
                if reviver_obj.is_some() {
                    let (val, node) = Self::parse_ecma404::<_, true>(bytes, context)?;
                    (val, Some(node))
                } else {
                    let (val, _) = Self::parse_ecma404::<_, false>(bytes, context)?;
                    (val, None)
                }
            }
            boa_string::JsStrVariant::Utf16(units) => {
                if reviver_obj.is_some() {
                    let (val, node) = Self::parse_ecma404::<_, true>(units, context)?;
                    (val, Some(node))
                } else {
                    let (val, _) = Self::parse_ecma404::<_, false>(units, context)?;
                    (val, None)
                }
            }
        };

        // 11. If IsCallable(reviver) is true, then
        if let Some(obj) = reviver_obj {
            // a. Let root be ! OrdinaryObjectCreate(%Object.prototype%).
            let root = JsObject::with_object_proto(context.intrinsics());

            // b. Let rootName be the empty String.
            // c. Perform ! CreateDataPropertyOrThrow(root, rootName, unfiltered).
            root.create_data_property_or_throw(js_string!(), unfiltered, context)
                .js_expect("CreateDataPropertyOrThrow should never throw here")?;

            // d. Return ? InternalizeJSONProperty(root, rootName, reviver).
            Self::internalize_json_property(
                &root,
                js_string!(),
                &obj,
                source_tree.as_ref(),
                context,
            )
        } else {
            // 12. Else,
            // a. Return unfiltered.
            Ok(unfiltered)
        }
    }

    /// Direct ECMA-404 recursive-descent JSON parser over `Latin1(&[u8])` or `Utf16(&[u16])`.
    ///
    /// - Uses `fast-float2` for exact IEEE-754 round-to-nearest-even `f64` conversion and `±Infinity` on overflow.
    /// - Preserves lone UTF-16 surrogates (`\uD800`–`\uDFFF` and literal unpaired surrogates).
    /// - Zero-copy `JsStr::latin1` / `utf16` slicing for unescaped strings.
    /// - Supports nesting up to 512 levels without stack overflow.
    fn parse_ecma404<'a, I: JsonInput<'a>, const WITH_SOURCE: bool>(
        input: I,
        context: &mut Context,
    ) -> JsResult<(JsValue, JsonNode)> {
        const MAX_DEPTH: usize = 512;

        struct ParserState<'a, I> {
            input: I,
            pos: usize,
            _marker: std::marker::PhantomData<&'a ()>,
        }

        impl<'a, I: JsonInput<'a>> ParserState<'a, I> {
            #[inline]
            fn syntax_err(&self, msg: &str) -> crate::JsError {
                if self.pos == 0 {
                    JsNativeError::syntax()
                        .with_message(format!("expected value at line 1 column 1: {msg}"))
                        .into()
                } else {
                    JsNativeError::syntax()
                        .with_message(format!("{msg} at position {}", self.pos))
                        .into()
                }
            }

            #[inline]
            fn skip_ws(&mut self) {
                let len = self.input.len();
                while self.pos < len {
                    match self.input.code_unit(self.pos) {
                        0x20 | 0x09 | 0x0A | 0x0D => self.pos += 1,
                        _ => break,
                    }
                }
            }

            #[inline]
            fn dummy_node() -> JsonNode {
                JsonNode::Primitive(js_string!(), JsValue::undefined())
            }

            fn parse_value<const SRC: bool>(
                &mut self,
                depth: usize,
                context: &mut Context,
            ) -> JsResult<(JsValue, JsonNode)> {
                if depth > MAX_DEPTH {
                    return Err(self.syntax_err("JSON nesting depth exceeded"));
                }
                self.skip_ws();
                if self.pos >= self.input.len() {
                    return Err(self.syntax_err("unexpected end of JSON input"));
                }
                match self.input.code_unit(self.pos) {
                    0x22 => {
                        let start = self.pos;
                        let s = self.parse_string()?;
                        let val = JsValue::new(s);
                        let node = if SRC {
                            JsonNode::Primitive(
                                self.input.slice_to_js_string(start, self.pos),
                                val.clone(),
                            )
                        } else {
                            Self::dummy_node()
                        };
                        Ok((val, node))
                    }
                    0x7B => self.parse_object::<SRC>(depth + 1, context),
                    0x5B => self.parse_array::<SRC>(depth + 1, context),
                    0x74 => {
                        if self.pos + 4 <= self.input.len()
                            && self.input.code_unit(self.pos + 1) == 0x72
                            && self.input.code_unit(self.pos + 2) == 0x75
                            && self.input.code_unit(self.pos + 3) == 0x65
                        {
                            self.pos += 4;
                            let val = JsValue::new(true);
                            let node = if SRC {
                                JsonNode::Primitive(js_string!("true"), val.clone())
                            } else {
                                Self::dummy_node()
                            };
                            Ok((val, node))
                        } else {
                            Err(self.syntax_err("expected 'true'"))
                        }
                    }
                    0x66 => {
                        if self.pos + 5 <= self.input.len()
                            && self.input.code_unit(self.pos + 1) == 0x61
                            && self.input.code_unit(self.pos + 2) == 0x6C
                            && self.input.code_unit(self.pos + 3) == 0x73
                            && self.input.code_unit(self.pos + 4) == 0x65
                        {
                            self.pos += 5;
                            let val = JsValue::new(false);
                            let node = if SRC {
                                JsonNode::Primitive(js_string!("false"), val.clone())
                            } else {
                                Self::dummy_node()
                            };
                            Ok((val, node))
                        } else {
                            Err(self.syntax_err("expected 'false'"))
                        }
                    }
                    0x6E => {
                        if self.pos + 4 <= self.input.len()
                            && self.input.code_unit(self.pos + 1) == 0x75
                            && self.input.code_unit(self.pos + 2) == 0x6C
                            && self.input.code_unit(self.pos + 3) == 0x6C
                        {
                            self.pos += 4;
                            let val = JsValue::null();
                            let node = if SRC {
                                JsonNode::Primitive(js_string!("null"), val.clone())
                            } else {
                                Self::dummy_node()
                            };
                            Ok((val, node))
                        } else {
                            Err(self.syntax_err("expected 'null'"))
                        }
                    }
                    0x2D | 0x30..=0x39 => {
                        let start = self.pos;
                        let val = self.parse_number()?;
                        let node = if SRC {
                            JsonNode::Primitive(
                                self.input.slice_to_js_string(start, self.pos),
                                val.clone(),
                            )
                        } else {
                            Self::dummy_node()
                        };
                        Ok((val, node))
                    }
                    _ => Err(self.syntax_err("unexpected token in JSON")),
                }
            }

            fn parse_string(&mut self) -> JsResult<JsString> {
                debug_assert_eq!(self.input.code_unit(self.pos), 0x22);
                self.pos += 1;
                let content_start = self.pos;
                let len = self.input.len();

                while self.pos < len {
                    let c = self.input.code_unit(self.pos);
                    if c == 0x22 {
                        let s = self.input.slice_to_js_string(content_start, self.pos);
                        self.pos += 1;
                        return Ok(s);
                    }
                    if c == 0x5C {
                        return self.parse_string_escaped(content_start);
                    }
                    if c < 0x20 {
                        return Err(self.syntax_err("unescaped control character in JSON string"));
                    }
                    self.pos += 1;
                }
                Err(self.syntax_err("unterminated string in JSON"))
            }

            fn parse_string_escaped(&mut self, content_start: usize) -> JsResult<JsString> {
                let len = self.input.len();
                let mut units = Vec::with_capacity((self.pos - content_start) + 16);
                for idx in content_start..self.pos {
                    units.push(self.input.code_unit(idx));
                }

                while self.pos < len {
                    let c = self.input.code_unit(self.pos);
                    if c == 0x22 {
                        self.pos += 1;
                        return Ok(js_string!(&units[..]));
                    }
                    if c < 0x20 {
                        return Err(self.syntax_err("unescaped control character in JSON string"));
                    }
                    if c == 0x5C {
                        self.pos += 1;
                        if self.pos >= len {
                            return Err(self.syntax_err("unterminated escape sequence in JSON"));
                        }
                        let esc = self.input.code_unit(self.pos);
                        self.pos += 1;
                        match esc {
                            0x22 => units.push(0x22),
                            0x5C => units.push(0x5C),
                            0x2F => units.push(0x2F),
                            0x62 => units.push(0x08),
                            0x66 => units.push(0x0C),
                            0x6E => units.push(0x0A),
                            0x72 => units.push(0x0D),
                            0x74 => units.push(0x09),
                            0x75 => {
                                if self.pos + 4 > len {
                                    return Err(
                                        self.syntax_err("incomplete unicode escape in JSON")
                                    );
                                }
                                let mut code: u16 = 0;
                                for _ in 0..4 {
                                    let hex = self.input.code_unit(self.pos);
                                    self.pos += 1;
                                    let nibble = match hex {
                                        0x30..=0x39 => hex - 0x30,
                                        0x61..=0x66 => hex - 0x61 + 10,
                                        0x41..=0x46 => hex - 0x41 + 10,
                                        _ => {
                                            return Err(self
                                                .syntax_err("invalid unicode hex digit in JSON"));
                                        }
                                    };
                                    code = (code << 4) | nibble;
                                }
                                units.push(code);
                            }
                            _ => return Err(self.syntax_err("invalid escape character in JSON")),
                        }
                    } else {
                        units.push(c);
                        self.pos += 1;
                    }
                }
                Err(self.syntax_err("unterminated string in JSON"))
            }

            fn parse_number(&mut self) -> JsResult<JsValue> {
                let start = self.pos;
                let len = self.input.len();
                let mut is_neg = false;
                if self.input.code_unit(self.pos) == 0x2D {
                    is_neg = true;
                    self.pos += 1;
                }
                if self.pos >= len {
                    return Err(self.syntax_err("unexpected end of number in JSON"));
                }
                match self.input.code_unit(self.pos) {
                    0x30 => {
                        self.pos += 1;
                        if self.pos < len && matches!(self.input.code_unit(self.pos), 0x30..=0x39) {
                            return Err(self.syntax_err("leading zero in JSON number"));
                        }
                    }
                    0x31..=0x39 => {
                        self.pos += 1;
                        while self.pos < len
                            && matches!(self.input.code_unit(self.pos), 0x30..=0x39)
                        {
                            self.pos += 1;
                        }
                    }
                    _ => return Err(self.syntax_err("expected digit in JSON number")),
                }

                let mut is_float = false;
                if self.pos < len && self.input.code_unit(self.pos) == 0x2E {
                    is_float = true;
                    self.pos += 1;
                    if self.pos >= len || !matches!(self.input.code_unit(self.pos), 0x30..=0x39) {
                        return Err(self.syntax_err("expected digit after decimal point in JSON"));
                    }
                    while self.pos < len && matches!(self.input.code_unit(self.pos), 0x30..=0x39) {
                        self.pos += 1;
                    }
                }

                if self.pos < len && matches!(self.input.code_unit(self.pos), 0x65 | 0x45) {
                    is_float = true;
                    self.pos += 1;
                    if self.pos < len && matches!(self.input.code_unit(self.pos), 0x2B | 0x2D) {
                        self.pos += 1;
                    }
                    if self.pos >= len || !matches!(self.input.code_unit(self.pos), 0x30..=0x39) {
                        return Err(self.syntax_err("expected digit in JSON exponent"));
                    }
                    while self.pos < len && matches!(self.input.code_unit(self.pos), 0x30..=0x39) {
                        self.pos += 1;
                    }
                }

                let num_bytes = self.input.ascii_bytes(start, self.pos);
                let bytes = num_bytes.as_ref();

                if !is_float {
                    if is_neg && bytes.len() == 2 && bytes[1] == b'0' {
                        return Ok(JsValue::new(-0.0f64));
                    }
                    if bytes.len() <= 10 {
                        let digits = if is_neg { &bytes[1..] } else { bytes };
                        let mut acc: i64 = 0;
                        for &b in digits {
                            acc = acc * 10 + i64::from(b - b'0');
                        }
                        if is_neg {
                            acc = -acc;
                        }
                        if let Ok(i) = i32::try_from(acc) {
                            return Ok(JsValue::new(i));
                        }
                        return Ok(JsValue::new(acc as f64));
                    }
                }

                let f: f64 = fast_float2::parse(bytes)
                    .map_err(|_| self.syntax_err("invalid number in JSON"))?;
                Ok(JsValue::new(f))
            }

            fn parse_array<const SRC: bool>(
                &mut self,
                depth: usize,
                context: &mut Context,
            ) -> JsResult<(JsValue, JsonNode)> {
                debug_assert_eq!(self.input.code_unit(self.pos), 0x5B);
                self.pos += 1;
                self.skip_ws();
                if self.pos < self.input.len() && self.input.code_unit(self.pos) == 0x5D {
                    self.pos += 1;
                    let arr = crate::builtins::Array::array_create(0, None, context)?;
                    let node = if SRC {
                        JsonNode::Array(Vec::new())
                    } else {
                        Self::dummy_node()
                    };
                    return Ok((arr.into(), node));
                }

                let mut elements = thin_vec::ThinVec::new();
                let mut child_nodes = if SRC { Vec::new() } else { Vec::new() };

                loop {
                    let (elem, child_node) = self.parse_value::<SRC>(depth, context)?;
                    elements.push(elem);
                    if SRC {
                        child_nodes.push(child_node);
                    }
                    self.skip_ws();
                    if self.pos >= self.input.len() {
                        return Err(self.syntax_err("unterminated array in JSON"));
                    }
                    match self.input.code_unit(self.pos) {
                        0x2C => {
                            self.pos += 1;
                            self.skip_ws();
                            if self.pos < self.input.len() && self.input.code_unit(self.pos) == 0x5D
                            {
                                return Err(self.syntax_err("trailing comma in JSON array"));
                            }
                        }
                        0x5D => {
                            self.pos += 1;
                            break;
                        }
                        _ => return Err(self.syntax_err("expected ',' or ']' in JSON array")),
                    }
                }

                let arr =
                    crate::builtins::Array::array_create(elements.len() as u64, None, context)?;
                arr.borrow_mut()
                    .properties_mut()
                    .override_indexed_properties(elements);
                let node = if SRC {
                    JsonNode::Array(child_nodes)
                } else {
                    Self::dummy_node()
                };
                Ok((arr.into(), node))
            }

            fn parse_object<const SRC: bool>(
                &mut self,
                depth: usize,
                context: &mut Context,
            ) -> JsResult<(JsValue, JsonNode)> {
                debug_assert_eq!(self.input.code_unit(self.pos), 0x7B);
                self.pos += 1;
                self.skip_ws();
                let obj = JsObject::with_object_proto(context.intrinsics());
                if self.pos < self.input.len() && self.input.code_unit(self.pos) == 0x7D {
                    self.pos += 1;
                    let node = if SRC {
                        JsonNode::Object(Vec::new())
                    } else {
                        Self::dummy_node()
                    };
                    return Ok((obj.into(), node));
                }

                let mut entries = if SRC { Vec::new() } else { Vec::new() };

                loop {
                    self.skip_ws();
                    if self.pos >= self.input.len() || self.input.code_unit(self.pos) != 0x22 {
                        return Err(self.syntax_err("expected string key in JSON object"));
                    }
                    let key = self.parse_string()?;
                    self.skip_ws();
                    if self.pos >= self.input.len() || self.input.code_unit(self.pos) != 0x3A {
                        return Err(self.syntax_err("expected ':' after key in JSON object"));
                    }
                    self.pos += 1;
                    let (val, child_node) = self.parse_value::<SRC>(depth, context)?;
                    if SRC {
                        entries.push((key.clone(), child_node));
                    }
                    obj.create_data_property_or_throw(key, val, context)?;
                    self.skip_ws();
                    if self.pos >= self.input.len() {
                        return Err(self.syntax_err("unterminated object in JSON"));
                    }
                    match self.input.code_unit(self.pos) {
                        0x2C => {
                            self.pos += 1;
                            self.skip_ws();
                            if self.pos < self.input.len() && self.input.code_unit(self.pos) == 0x7D
                            {
                                return Err(self.syntax_err("trailing comma in JSON object"));
                            }
                        }
                        0x7D => {
                            self.pos += 1;
                            break;
                        }
                        _ => return Err(self.syntax_err("expected ',' or '}' in JSON object")),
                    }
                }

                let node = if SRC {
                    JsonNode::Object(entries)
                } else {
                    Self::dummy_node()
                };
                Ok((obj.into(), node))
            }
        }

        let mut state = ParserState {
            input,
            pos: 0,
            _marker: std::marker::PhantomData,
        };
        let res = state.parse_value::<WITH_SOURCE>(0, context)?;
        state.skip_ws();
        if state.pos != state.input.len() {
            return Err(state.syntax_err("unexpected trailing characters in JSON"));
        }
        Ok(res)
    }

    /// `25.5.1.1 InternalizeJSONProperty ( holder, name, reviver )`
    ///
    /// More information:
    ///  - [ECMAScript reference][spec]
    ///
    /// [spec]: https://tc39.es/ecma262/#sec-internalizejsonproperty
    fn internalize_json_property(
        holder: &JsObject,
        name: JsString,
        reviver: &JsObject,
        source_node: Option<&JsonNode>,
        context: &mut Context,
    ) -> JsResult<JsValue> {
        // 1. Let val be ? Get(holder, name).
        let val = holder.get(name.clone(), context)?;

        // 2. If Type(val) is Object, then
        if let Some(obj) = val.as_object() {
            // a. Let isArray be ? IsArray(val).
            // b. If isArray is true, then
            if obj.is_array_abstract()? {
                // i. Let I be 0.
                // ii. Let len be ? LengthOfArrayLike(val).
                // iii. Repeat, while I < len,
                let len = obj.length_of_array_like(context)? as i64;
                let children = match source_node {
                    Some(JsonNode::Array(children)) => Some(children),
                    _ => None,
                };
                for i in 0..len {
                    let child_node = children.and_then(|c| c.get(i as usize));
                    // 1. Let prop be ! ToString(𝔽(I)).
                    // 2. Let newElement be ? InternalizeJSONProperty(val, prop, reviver).
                    let new_element = Self::internalize_json_property(
                        &obj,
                        i.into(),
                        reviver,
                        child_node,
                        context,
                    )?;

                    // 3. If newElement is undefined, then
                    if new_element.is_undefined() {
                        // a. Perform ? val.[[Delete]](prop).
                        obj.__delete__(
                            &i.into(),
                            &mut InternalMethodPropertyContext::new(context),
                        )?;
                    }
                    // 4. Else,
                    else {
                        // a. Perform ? CreateDataProperty(val, prop, newElement).
                        obj.create_data_property(i, new_element, context)?;
                    }
                }
            }
            // c. Else,
            else {
                // i. Let keys be ? EnumerableOwnPropertyNames(val, key).
                let keys = obj.enumerable_own_property_names(PropertyNameKind::Key, context)?;
                let entries = match source_node {
                    Some(JsonNode::Object(entries)) => Some(entries),
                    _ => None,
                };

                // ii. For each String P of keys, do
                for p in keys {
                    // This is safe, because EnumerableOwnPropertyNames with 'key' type only returns strings.
                    let p = p
                        .as_string()
                        .js_expect("EnumerableOwnPropertyNames only returns strings")?;

                    let child_node =
                        entries.and_then(|e| e.iter().rfind(|(k, _)| k == &p).map(|(_, v)| v));

                    // 1. Let newElement be ? InternalizeJSONProperty(val, P, reviver).
                    let new_element = Self::internalize_json_property(
                        &obj,
                        p.clone(),
                        reviver,
                        child_node,
                        context,
                    )?;

                    // 2. If newElement is undefined, then
                    if new_element.is_undefined() {
                        // a. Perform ? val.[[Delete]](P).
                        obj.__delete__(
                            &p.into(),
                            &mut InternalMethodPropertyContext::new(context),
                        )?;
                    }
                    // 3. Else,
                    else {
                        // a. Perform ? CreateDataProperty(val, P, newElement).
                        obj.create_data_property(p, new_element, context)?;
                    }
                }
            }
        }

        // Build the context object for the reviver call.
        let ctx_obj = JsObject::with_object_proto(context.intrinsics());
        if let Some(JsonNode::Primitive(source_text, original_val)) = source_node
            && JsValue::same_value(&val, original_val)
        {
            ctx_obj.create_data_property_or_throw(
                js_string!("source"),
                JsValue::from(source_text.clone()),
                context,
            )?;
        }

        // 3. Return ? Call(reviver, holder, « name, val, context »).
        reviver.call(
            &holder.clone().into(),
            &[name.into(), val, ctx_obj.into()],
            context,
        )
    }

    /// `JSON.rawJSON ( text )`
    ///
    /// Creates a raw JSON object from the given text.
    ///
    /// More information:
    ///  - [ECMAScript reference][spec]
    ///
    /// [spec]: https://tc39.es/ecma262/#sec-json.rawjson
    pub(crate) fn raw_json(
        _: &JsValue,
        args: &[JsValue],
        context: &mut Context,
    ) -> JsResult<JsValue> {
        // 1. Let jsonString be ? ToString(text).
        let json_string = args.get_or_undefined(0).to_string(context)?;

        // 2. Throw a SyntaxError exception if jsonString is the empty String, or if
        //    either the first or last code unit of jsonString is any of 0x0009
        //    (CHARACTER TABULATION), 0x000A (LINE FEED), 0x000D (CARRIAGE RETURN), or
        //    0x0020 (SPACE).
        if json_string.is_empty() {
            return Err(JsNativeError::syntax()
                .with_message("JSON.rawJSON text must not be the empty string")
                .into());
        }

        let first = json_string.code_unit_at(0).unwrap_or(0);
        let last = json_string.code_unit_at(json_string.len() - 1).unwrap_or(0);
        if matches!(first, 0x09 | 0x0A | 0x0D | 0x20) || matches!(last, 0x09 | 0x0A | 0x0D | 0x20) {
            return Err(JsNativeError::syntax()
                .with_message("JSON.rawJSON text must not start or end with whitespace")
                .into());
        }
        if matches!(first, 0x7B | 0x5B) {
            return Err(JsNativeError::syntax()
                .with_message("JSON.rawJSON text must not be an object or array")
                .into());
        }

        // 3. Parse StringToCodePoints(jsonString) as a JSON text as specified in ECMA-404.
        match json_string.as_str().variant() {
            boa_string::JsStrVariant::Latin1(bytes) => {
                Self::parse_ecma404::<_, false>(bytes, context)?;
            }
            boa_string::JsStrVariant::Utf16(units) => {
                Self::parse_ecma404::<_, false>(units, context)?;
            }
        }

        // 3. Let internalSlotsList be « [[IsRawJSON]] ».
        // 4. Let obj be OrdinaryObjectCreate(null, internalSlotsList).
        let obj = JsObject::from_proto_and_data(None::<JsObject>, RawJson);

        // 5. Perform ! CreateDataPropertyOrThrow(obj, "rawJSON", jsonString).
        obj.create_data_property_or_throw(js_string!("rawJSON"), json_string, context)
            .js_expect("CreateDataPropertyOrThrow should never throw here")?;

        // 6. Perform ! SetIntegrityLevel(obj, frozen).
        obj.set_integrity_level(IntegrityLevel::Frozen, context)
            .js_expect("SetIntegrityLevel should never throw here")?;

        // 7. Return obj.
        Ok(obj.into())
    }

    /// `JSON.isRawJSON ( O )`
    ///
    /// Checks if the given value is a raw JSON object.
    ///
    /// More information:
    ///  - [ECMAScript reference][spec]
    ///
    /// [spec]: https://tc39.es/ecma262/#sec-json.israwjson
    #[allow(clippy::unnecessary_wraps)]
    pub(crate) fn is_raw_json(
        _: &JsValue,
        args: &[JsValue],
        _context: &mut Context,
    ) -> JsResult<JsValue> {
        // 1. If Type(O) is Object and O has an [[IsRawJSON]] internal slot, return true.
        // 2. Return false.
        let value = args.get_or_undefined(0);
        let result = value.as_object().is_some_and(|obj| obj.is::<RawJson>());
        Ok(result.into())
    }

    /// `JSON.stringify( value[, replacer[, space]] )`
    ///
    /// This `JSON` method converts a JavaScript object or value to a JSON string.
    ///
    /// This method optionally replaces values if a `replacer` function is specified or
    /// optionally including only the specified properties if a replacer array is specified.
    ///
    /// An optional `space` argument can be supplied of type `String` or `Number` that's used to insert
    /// white space into the output JSON string for readability purposes.
    ///
    /// More information:
    ///  - [ECMAScript reference][spec]
    ///  - [MDN documentation][mdn]
    ///
    /// [spec]: https://tc39.es/ecma262/#sec-json.stringify
    /// [mdn]: https://developer.mozilla.org/en-US/docs/Web/JavaScript/Reference/Global_Objects/JSON/stringify
    pub(crate) fn stringify(
        _: &JsValue,
        args: &[JsValue],
        context: &mut Context,
    ) -> JsResult<JsValue> {
        // 1. Let indent be the empty String.
        let indent = js_string!();

        // 2. Let PropertyList and ReplacerFunction be undefined.
        let mut property_list = None;
        let mut replacer_function = None;

        let replacer = args.get_or_undefined(1);

        // 4. If Type(replacer) is Object, then
        if let Some(replacer_obj) = replacer.as_object() {
            // a. If IsCallable(replacer) is true, then
            if replacer_obj.is_callable() {
                // i. Set ReplacerFunction to replacer.
                replacer_function = Some(replacer_obj.clone());
            // b. Else,
            } else {
                // i. Let isArray be ? IsArray(replacer).
                // ii. If isArray is true, then
                if replacer_obj.is_array_abstract()? {
                    // 1. Set PropertyList to a new empty List.
                    let mut property_set = indexmap::IndexSet::new();

                    // 2. Let len be ? LengthOfArrayLike(replacer).
                    let len = replacer_obj.length_of_array_like(context)?;

                    // 3. Let k be 0.
                    let mut k = 0;

                    // 4. Repeat, while k < len,
                    while k < len {
                        // a. Let prop be ! ToString(𝔽(k)).
                        // b. Let v be ? Get(replacer, prop).
                        let v = replacer_obj.get(k, context)?;

                        // c. Let item be undefined.
                        // d. If Type(v) is String, set item to v.
                        // e. Else if Type(v) is Number, set item to ! ToString(v).
                        // f. Else if Type(v) is Object, then
                        // g. If item is not undefined and item is not currently an element of PropertyList, then
                        // i. Append item to the end of PropertyList.
                        if let Some(s) = v.as_string() {
                            property_set.insert(s);
                        } else if v.is_number() {
                            property_set.insert(
                                v.to_string(context)
                                    .js_expect("ToString cannot fail on number value")?,
                            );
                        } else if let Some(obj) = v.as_object()
                            && (obj.is::<JsString>() || obj.is::<f64>())
                        {
                            // i. If v has a [[StringData]] or [[NumberData]] internal slot, set item to ? ToString(v).
                            property_set.insert(v.to_string(context)?);
                        }

                        // h. Set k to k + 1.
                        k += 1;
                    }
                    property_list = Some(property_set.into_iter().collect());
                }
            }
        }

        let mut space = args.get_or_undefined(2).clone();

        // 5. If Type(space) is Object, then
        if let Some(space_obj) = space.as_object() {
            // a. If space has a [[NumberData]] internal slot, then
            if space_obj.is::<f64>() {
                // i. Set space to ? ToNumber(space).
                space = space.to_number(context)?.into();
            }
            // b. Else if space has a [[StringData]] internal slot, then
            else if space_obj.is::<JsString>() {
                // i. Set space to ? ToString(space).
                space = space.to_string(context)?.into();
            }
        }

        // 6. If Type(space) is Number, then
        let gap = if space.is_number() {
            // a. Let spaceMV be ! ToIntegerOrInfinity(space).
            // b. Set spaceMV to min(10, spaceMV).
            // c. If spaceMV < 1, let gap be the empty String; otherwise let gap be the String value containing spaceMV occurrences of the code unit 0x0020 (SPACE).
            match space
                .to_integer_or_infinity(context)
                .js_expect("ToIntegerOrInfinity cannot fail on number")?
            {
                IntegerOrInfinity::PositiveInfinity => js_string!("          "),
                IntegerOrInfinity::NegativeInfinity => js_string!(),
                IntegerOrInfinity::Integer(i) if i < 1 => js_string!(),
                IntegerOrInfinity::Integer(i) => {
                    let mut s = String::new();
                    let i = std::cmp::min(10, i);
                    for _ in 0..i {
                        s.push(' ');
                    }
                    s.into()
                }
            }
        // 7. Else if Type(space) is String, then
        } else if let Some(s) = space.as_string() {
            // a. If the length of space is 10 or less, let gap be space; otherwise let gap be the substring of space from 0 to 10.
            s.get(..10).unwrap_or(s)
        // 8. Else,
        } else {
            // a. Let gap be the empty String.
            js_string!()
        };

        // SLICE PATCH (P03): Single-buffer `try_fast_stringify` when no `replacer` or `gap` is supplied.
        if replacer_function.is_none() && property_list.is_none() && gap.is_empty() {
            if let Some(fast_val) = Self::try_fast_stringify(args.get_or_undefined(0), context)? {
                return Ok(fast_val);
            }
        }

        // 9. Let wrapper be ! OrdinaryObjectCreate(%Object.prototype%).
        let wrapper = JsObject::with_object_proto(context.intrinsics());

        // 10. Perform ! CreateDataPropertyOrThrow(wrapper, the empty String, value).
        wrapper
            .create_data_property_or_throw(js_string!(), args.get_or_undefined(0).clone(), context)
            .js_expect("CreateDataPropertyOrThrow should never fail here")?;

        // 11. Let state be the Record { [[ReplacerFunction]]: ReplacerFunction, [[Stack]]: stack, [[Indent]]: indent, [[Gap]]: gap, [[PropertyList]]: PropertyList }.
        let mut state = StateRecord {
            replacer_function,
            stack_set: FxHashSet::default(),
            indent,
            gap,
            property_list,
        };

        // 12. Return ? SerializeJSONProperty(state, the empty String, wrapper).
        Ok(
            Self::serialize_json_property(&mut state, js_string!(), &wrapper, context)?
                .map(Into::into)
                .unwrap_or_default(),
        )
    }

    /// SLICE PATCH (P03): Direct single-buffer JSON serializer for ordinary objects, dense arrays,
    /// strings, numbers, booleans, and `null` when no `replacer` or `space` is provided.
    ///
    /// Eliminates wrapper object creation, per-property `get_v("toJSON")` prototype walks,
    /// `enumerable_own_property_names` `Vec<JsValue>` + `Vec<JsString>` allocations, and per-string
    /// `Vec<u16>` allocations. Falls back (`Ok(None)`) whenever any object has `toJSON`, accessors,
    /// non-standard prototypes, indexed properties on non-arrays, sparse arrays, or exotic slots.
    fn try_fast_stringify(value: &JsValue, context: &mut Context) -> JsResult<Option<JsValue>> {
        use crate::object::{IndexedProperties, shape::slot::SlotAttributes};
        use crate::property::PropertyKey;
        use crate::value::JsVariant;

        enum FastStatus {
            Written,
            UndefinedOrSkipped,
            Fallback,
        }

        #[inline]
        fn write_i32(v: i32, buf: &mut Vec<u8>) {
            if v == 0 {
                buf.push(b'0');
                return;
            }
            let mut n = i64::from(v);
            if n < 0 {
                buf.push(b'-');
                n = -n;
            }
            let mut tmp = [0u8; 12];
            let mut pos = tmp.len();
            while n > 0 {
                pos -= 1;
                tmp[pos] = b'0' + (n % 10) as u8;
                n /= 10;
            }
            buf.extend_from_slice(&tmp[pos..]);
        }

        #[inline]
        fn write_f64(n: f64, buf: &mut Vec<u8>) {
            if n.is_finite() {
                let mut ryu_buf = ryu_js::Buffer::new();
                buf.extend_from_slice(ryu_buf.format_finite(n).as_bytes());
            } else {
                buf.extend_from_slice(b"null");
            }
        }

        #[inline]
        fn write_js_str(s: &JsString, buf: &mut Vec<u8>, all_ascii: &mut bool) -> bool {
            const HEX: &[u8; 16] = b"0123456789abcdef";
            match s.as_str().variant() {
                boa_string::JsStrVariant::Latin1(bytes) => {
                    buf.push(b'"');
                    let mut start = 0usize;
                    for (i, &b) in bytes.iter().enumerate() {
                        if b < 0x20 || b == b'"' || b == b'\\' || b >= 0x80 {
                            if i > start {
                                buf.extend_from_slice(&bytes[start..i]);
                            }
                            match b {
                                b'"' => buf.extend_from_slice(b"\\\""),
                                b'\\' => buf.extend_from_slice(b"\\\\"),
                                0x08 => buf.extend_from_slice(b"\\b"),
                                0x09 => buf.extend_from_slice(b"\\t"),
                                0x0A => buf.extend_from_slice(b"\\n"),
                                0x0C => buf.extend_from_slice(b"\\f"),
                                0x0D => buf.extend_from_slice(b"\\r"),
                                b if b < 0x20 => {
                                    buf.extend_from_slice(&[
                                        b'\\',
                                        b'u',
                                        b'0',
                                        b'0',
                                        HEX[(b >> 4) as usize],
                                        HEX[(b & 0xF) as usize],
                                    ]);
                                }
                                b => {
                                    *all_ascii = false;
                                    buf.push(0xC0 | (b >> 6));
                                    buf.push(0x80 | (b & 0x3F));
                                }
                            }
                            start = i + 1;
                        }
                    }
                    if start < bytes.len() {
                        buf.extend_from_slice(&bytes[start..]);
                    }
                    buf.push(b'"');
                    true
                }
                boa_string::JsStrVariant::Utf16(units) => {
                    buf.push(b'"');
                    let mut utf8_tmp = [0u8; 4];
                    for decoded in char::decode_utf16(units.iter().copied()) {
                        let Ok(c) = decoded else {
                            return false;
                        };
                        match c {
                            '"' => buf.extend_from_slice(b"\\\""),
                            '\\' => buf.extend_from_slice(b"\\\\"),
                            '\u{0008}' => buf.extend_from_slice(b"\\b"),
                            '\u{0009}' => buf.extend_from_slice(b"\\t"),
                            '\u{000A}' => buf.extend_from_slice(b"\\n"),
                            '\u{000C}' => buf.extend_from_slice(b"\\f"),
                            '\u{000D}' => buf.extend_from_slice(b"\\r"),
                            c if c < '\u{0020}' => {
                                let b = c as u8;
                                buf.extend_from_slice(&[
                                    b'\\',
                                    b'u',
                                    b'0',
                                    b'0',
                                    HEX[(b >> 4) as usize],
                                    HEX[(b & 0xF) as usize],
                                ]);
                            }
                            c if c.is_ascii() => {
                                buf.push(c as u8);
                            }
                            c => {
                                *all_ascii = false;
                                let enc = c.encode_utf8(&mut utf8_tmp);
                                buf.extend_from_slice(enc.as_bytes());
                            }
                        }
                    }
                    buf.push(b'"');
                    true
                }
            }
        }

        #[allow(clippy::too_many_arguments)]
        fn write_val(
            val: &JsValue,
            buf: &mut Vec<u8>,
            all_ascii: &mut bool,
            stack: &mut Vec<usize>,
            obj_proto: &JsObject,
            arr_proto: &JsObject,
            arr_template_shape_addr: usize,
            to_json_key: &PropertyKey,
        ) -> JsResult<FastStatus> {
            match val.variant() {
                JsVariant::Null => {
                    buf.extend_from_slice(b"null");
                    Ok(FastStatus::Written)
                }
                JsVariant::Boolean(b) => {
                    buf.extend_from_slice(if b { b"true" } else { b"false" });
                    Ok(FastStatus::Written)
                }
                JsVariant::Integer32(i) => {
                    write_i32(i, buf);
                    Ok(FastStatus::Written)
                }
                JsVariant::Float64(f) => {
                    write_f64(f, buf);
                    Ok(FastStatus::Written)
                }
                JsVariant::String(s) => {
                    if write_js_str(&s, buf, all_ascii) {
                        Ok(FastStatus::Written)
                    } else {
                        Ok(FastStatus::Fallback)
                    }
                }
                JsVariant::Undefined | JsVariant::Symbol(_) => Ok(FastStatus::UndefinedOrSkipped),
                JsVariant::BigInt(_) => Ok(FastStatus::Fallback),
                JsVariant::Object(obj) => {
                    let borrow = obj.borrow();
                    let props = borrow.properties();
                    if props.shape.lookup(to_json_key).is_some() {
                        return Ok(FastStatus::Fallback);
                    }
                    if obj.is_callable() {
                        return Ok(FastStatus::UndefinedOrSkipped);
                    }
                    let addr = std::ptr::from_ref(obj.as_ref()).addr();
                    if stack.contains(&addr) {
                        return Err(JsNativeError::typ()
                            .with_message("cyclic object value")
                            .into());
                    }

                    if obj.is_array() {
                        if props.shape.to_addr_usize() != arr_template_shape_addr {
                            if let Some(ref p) = props.shape.prototype() {
                                if p != arr_proto {
                                    return Ok(FastStatus::Fallback);
                                }
                            } else {
                                return Ok(FastStatus::Fallback);
                            }
                        }
                        let Some(len_val) = props.storage.first() else {
                            return Ok(FastStatus::Fallback);
                        };
                        let arr_len = match len_val.variant() {
                            JsVariant::Integer32(i) if i >= 0 => i as usize,
                            JsVariant::Float64(f) if f >= 0.0 && f <= (u32::MAX as f64) => {
                                f as usize
                            }
                            _ => return Ok(FastStatus::Fallback),
                        };
                        match &props.indexed_properties {
                            IndexedProperties::DenseI32(vec) if vec.len() == arr_len => {
                                buf.push(b'[');
                                for (idx, &v) in vec.iter().enumerate() {
                                    if idx > 0 {
                                        buf.push(b',');
                                    }
                                    write_i32(v, buf);
                                }
                                buf.push(b']');
                                return Ok(FastStatus::Written);
                            }
                            IndexedProperties::DenseF64(vec) if vec.len() == arr_len => {
                                buf.push(b'[');
                                for (idx, &v) in vec.iter().enumerate() {
                                    if idx > 0 {
                                        buf.push(b',');
                                    }
                                    write_f64(v, buf);
                                }
                                buf.push(b']');
                                return Ok(FastStatus::Written);
                            }
                            IndexedProperties::DenseElement(vec) if vec.len() == arr_len => {
                                stack.push(addr);
                                buf.push(b'[');
                                for (idx, elem) in vec.iter().enumerate() {
                                    if idx > 0 {
                                        buf.push(b',');
                                    }
                                    match write_val(
                                        elem,
                                        buf,
                                        all_ascii,
                                        stack,
                                        obj_proto,
                                        arr_proto,
                                        arr_template_shape_addr,
                                        to_json_key,
                                    ) {
                                        Ok(FastStatus::Written) => {}
                                        Ok(FastStatus::UndefinedOrSkipped) => {
                                            buf.extend_from_slice(b"null");
                                        }
                                        Ok(FastStatus::Fallback) => {
                                            stack.pop();
                                            return Ok(FastStatus::Fallback);
                                        }
                                        Err(e) => {
                                            stack.pop();
                                            return Err(e);
                                        }
                                    }
                                }
                                buf.push(b']');
                                stack.pop();
                                return Ok(FastStatus::Written);
                            }
                            _ if arr_len == 0 => {
                                buf.extend_from_slice(b"[]");
                                return Ok(FastStatus::Written);
                            }
                            _ => return Ok(FastStatus::Fallback),
                        }
                    }

                    let indexed_empty = match &props.indexed_properties {
                        IndexedProperties::DenseI32(v) => v.is_empty(),
                        IndexedProperties::DenseF64(v) => v.is_empty(),
                        IndexedProperties::DenseElement(v) => v.is_empty(),
                        IndexedProperties::SparseElement(m) => m.is_empty(),
                        IndexedProperties::SparseProperty(m) => m.is_empty(),
                    };
                    if !obj.is_ordinary() || !indexed_empty {
                        return Ok(FastStatus::Fallback);
                    }
                    if let Some(ref p) = props.shape.prototype() {
                        if p != obj_proto {
                            return Ok(FastStatus::Fallback);
                        }
                    }

                    stack.push(addr);
                    let res = props
                        .shape
                        .with_property_keys(|keys| -> JsResult<FastStatus> {
                            buf.push(b'{');
                            let mut first = true;
                            for (key, slot) in keys {
                                if slot.attributes.is_accessor_descriptor() {
                                    return Ok(FastStatus::Fallback);
                                }
                                if !slot.attributes.contains(SlotAttributes::ENUMERABLE) {
                                    continue;
                                }
                                let PropertyKey::String(key_str) = key else {
                                    continue;
                                };
                                let prop_val = &props.storage[slot.index as usize];
                                match prop_val.variant() {
                                    JsVariant::Undefined | JsVariant::Symbol(_) => continue,
                                    JsVariant::Object(ref child_obj) if child_obj.is_callable() => {
                                        if child_obj
                                            .borrow()
                                            .properties()
                                            .shape
                                            .lookup(to_json_key)
                                            .is_some()
                                        {
                                            return Ok(FastStatus::Fallback);
                                        }
                                        continue;
                                    }
                                    _ => {}
                                }
                                if !first {
                                    buf.push(b',');
                                }
                                if !write_js_str(key_str, buf, all_ascii) {
                                    return Ok(FastStatus::Fallback);
                                }
                                buf.push(b':');
                                match write_val(
                                    prop_val,
                                    buf,
                                    all_ascii,
                                    stack,
                                    obj_proto,
                                    arr_proto,
                                    arr_template_shape_addr,
                                    to_json_key,
                                )? {
                                    FastStatus::Written => {
                                        first = false;
                                    }
                                    FastStatus::UndefinedOrSkipped | FastStatus::Fallback => {
                                        return Ok(FastStatus::Fallback);
                                    }
                                }
                            }
                            buf.push(b'}');
                            Ok(FastStatus::Written)
                        });
                    stack.pop();
                    res
                }
            }
        }

        let to_json_key = PropertyKey::from(js_string!("toJSON"));
        let obj_proto = context.intrinsics().constructors().object().prototype();
        let arr_proto = context.intrinsics().constructors().array().prototype();
        if obj_proto
            .borrow()
            .properties()
            .shape
            .lookup(&to_json_key)
            .is_some()
            || arr_proto
                .borrow()
                .properties()
                .shape
                .lookup(&to_json_key)
                .is_some()
        {
            return Ok(None);
        }
        let arr_template_shape_addr = context
            .intrinsics()
            .templates()
            .array()
            .shape()
            .to_addr_usize();

        let mut buf = Vec::with_capacity(128);
        let mut all_ascii = true;
        let mut stack = Vec::with_capacity(8);

        match write_val(
            value,
            &mut buf,
            &mut all_ascii,
            &mut stack,
            &obj_proto,
            &arr_proto,
            arr_template_shape_addr,
            &to_json_key,
        )? {
            FastStatus::Written => {
                if all_ascii {
                    Ok(Some(JsValue::new(JsString::from(
                        boa_string::JsStr::latin1(&buf),
                    ))))
                } else {
                    let s = std::str::from_utf8(&buf).unwrap_or("");
                    Ok(Some(JsValue::new(js_string!(s))))
                }
            }
            FastStatus::UndefinedOrSkipped => Ok(Some(JsValue::undefined())),
            FastStatus::Fallback => Ok(None),
        }
    }

    /// `25.5.2.1 SerializeJSONProperty ( state, key, holder )`
    ///
    /// More information:
    ///  - [ECMAScript reference][spec]
    ///
    /// [spec]: https://tc39.es/ecma262/#sec-serializejsonproperty
    fn serialize_json_property(
        state: &mut StateRecord,
        key: JsString,
        holder: &JsObject,
        context: &mut Context,
    ) -> JsResult<Option<JsString>> {
        // 1. Let value be ? Get(holder, key).
        let mut value = holder.get(key.clone(), context)?;

        // 2. If Type(value) is Object or BigInt, then
        if value.is_object() || value.is_bigint() {
            // a. Let toJSON be ? GetV(value, "toJSON").
            let to_json = value.get_v(js_string!("toJSON"), context)?;

            // b. If IsCallable(toJSON) is true, then
            if let Some(obj) = to_json.as_callable() {
                // i. Set value to ? Call(toJSON, value, « key »).
                value = obj.call(&value, &[key.clone().into()], context)?;
            }
        }

        // 3. If state.[[ReplacerFunction]] is not undefined, then
        if let Some(obj) = &state.replacer_function {
            // a. Set value to ? Call(state.[[ReplacerFunction]], holder, « key, value »).
            value = obj.call(&holder.clone().into(), &[key.into(), value], context)?;
        }

        // 4. If Type(value) is Object, then
        if let Some(obj) = value.as_object() {
            // a. If value has a [[NumberData]] internal slot, then
            if obj.is::<f64>() {
                // i. Set value to ? ToNumber(value).
                value = value.to_number(context)?.into();
            }
            // b. Else if value has a [[StringData]] internal slot, then
            else if obj.is::<JsString>() {
                // i. Set value to ? ToString(value).
                value = value.to_string(context)?.into();
            }
            // c. Else if value has a [[BooleanData]] internal slot, then
            else if let Some(boolean) = obj.downcast_ref::<bool>() {
                // i. Set value to value.[[BooleanData]].
                value = (*boolean).into();
            }
            // d. Else if value has a [[BigIntData]] internal slot, then
            else if let Some(bigint) = obj.downcast_ref::<JsBigInt>() {
                // i. Set value to value.[[BigIntData]].
                value = bigint.clone().into();
            }
            // e. Else if value has a [[IsRawJSON]] internal slot, then
            else if obj.is::<RawJson>() {
                // i. Return the value of the "rawJSON" property of value.
                let raw = obj.get(js_string!("rawJSON"), context)?;
                return Ok(raw.as_string());
            }
        }

        // 5. If value is null, return "null".
        if value.is_null() {
            return Ok(Some(js_string!("null")));
        }

        // 6. If value is true, return "true".
        // 7. If value is false, return "false".
        if value.is_boolean() {
            return Ok(Some(js_string!(if value.to_boolean() {
                "true"
            } else {
                "false"
            })));
        }

        // 8. If Type(value) is String, return QuoteJSONString(value).
        if let Some(s) = value.as_string() {
            return Ok(Some(Self::quote_json_string(&s)));
        }

        // 9. If Type(value) is Number, then
        if let Some(n) = value.as_number() {
            // a. If value is finite, return ! ToString(value).
            if n.is_finite() {
                return Ok(Some(
                    value
                        .to_string(context)
                        .js_expect("ToString should never fail here")?,
                ));
            }

            // b. Return "null".
            return Ok(Some(js_string!("null")));
        }

        // 10. If Type(value) is BigInt, throw a TypeError exception.
        if value.is_bigint() {
            return Err(JsNativeError::typ()
                .with_message("cannot serialize bigint to JSON")
                .into());
        }

        // 11. If Type(value) is Object and IsCallable(value) is false, then
        if let Some(obj) = value.as_object()
            && !obj.is_callable()
        {
            // a. Let isArray be ? IsArray(value).
            // b. If isArray is true, return ? SerializeJSONArray(state, value).
            // c. Return ? SerializeJSONObject(state, value).
            return if obj.is_array_abstract()? {
                Ok(Some(Self::serialize_json_array(state, &obj, context)?))
            } else {
                Ok(Some(Self::serialize_json_object(state, &obj, context)?))
            };
        }

        // 12. Return undefined.
        Ok(None)
    }

    /// `25.5.2.2 QuoteJSONString ( value )`
    ///
    /// More information:
    ///  - [ECMAScript reference][spec]
    ///
    /// [spec]: https://tc39.es/ecma262/#sec-quotejsonstring
    fn quote_json_string(value: &JsString) -> JsString {
        let mut buf = [0; 2];
        // 1. Let product be the String value consisting solely of the code unit 0x0022 (QUOTATION MARK).
        let mut product = Vec::with_capacity(value.len() + 2);
        product.push('"' as u16);

        // 2. For each code point C of ! StringToCodePoints(value), do
        for code_point in value.code_points() {
            match code_point {
                // a. If C is listed in the "Code Point" column of Table 73, then
                // i. Set product to the string-concatenation of product and the
                // escape sequence for C as specified in the "Escape Sequence"
                // column of the corresponding row.
                CodePoint::Unicode('\u{0008}') => product.extend_from_slice(utf16!(r"\b")),
                CodePoint::Unicode('\u{0009}') => product.extend_from_slice(utf16!(r"\t")),
                CodePoint::Unicode('\u{000A}') => product.extend_from_slice(utf16!(r"\n")),
                CodePoint::Unicode('\u{000C}') => product.extend_from_slice(utf16!(r"\f")),
                CodePoint::Unicode('\u{000D}') => product.extend_from_slice(utf16!(r"\r")),
                CodePoint::Unicode('\u{0022}') => product.extend_from_slice(utf16!(r#"\""#)),
                CodePoint::Unicode('\u{005C}') => product.extend_from_slice(utf16!(r"\\")),
                // b. Else if C has a numeric value less than 0x0020 (SPACE), or
                // if C has the same numeric value as a leading surrogate or
                // trailing surrogate, then
                //     i. Let unit be the code unit whose numeric value is that
                //     of C.
                //     ii. Set product to the string-concatenation of product
                //     and UnicodeEscape(unit).
                CodePoint::Unicode(c) if c < '\u{0020}' => {
                    let val = c as u16;
                    product.extend_from_slice(&[
                        '\\' as u16,
                        'u' as u16,
                        to_hex_digit(val >> 12),
                        to_hex_digit((val >> 8) & 0xF),
                        to_hex_digit((val >> 4) & 0xF),
                        to_hex_digit(val & 0xF),
                    ]);
                }
                CodePoint::UnpairedSurrogate(surr) => {
                    product.extend_from_slice(&[
                        '\\' as u16,
                        'u' as u16,
                        to_hex_digit(surr >> 12),
                        to_hex_digit((surr >> 8) & 0xF),
                        to_hex_digit((surr >> 4) & 0xF),
                        to_hex_digit(surr & 0xF),
                    ]);
                }
                // c. Else,
                CodePoint::Unicode(c) => {
                    // i. Set product to the string-concatenation of product and ! UTF16EncodeCodePoint(C).
                    product.extend_from_slice(c.encode_utf16(&mut buf));
                }
            }
        }

        // 3. Set product to the string-concatenation of product and the code unit 0x0022 (QUOTATION MARK).
        product.push('"' as u16);

        // 4. Return product.
        js_string!(&product[..])
    }

    /// `25.5.2.4 SerializeJSONObject ( state, value )`
    ///
    /// More information:
    ///  - [ECMAScript reference][spec]
    ///
    /// [spec]: https://tc39.es/ecma262/#sec-serializejsonobject
    fn serialize_json_object(
        state: &mut StateRecord,
        value: &JsObject,
        context: &mut Context,
    ) -> JsResult<JsString> {
        // 1. If state.[[Stack]] contains value, throw a TypeError exception because the structure is cyclical.
        if !state.stack_set.insert(value.clone()) {
            return Err(JsNativeError::typ()
                .with_message("cyclic object value")
                .into());
        }

        // 3. Let stepback be state.[[Indent]].
        let stepback = state.indent.clone();

        // 4. Set state.[[Indent]] to the string-concatenation of state.[[Indent]] and state.[[Gap]].
        state.indent = js_string!(&state.indent, &state.gap);

        // 5. If state.[[PropertyList]] is not undefined, then
        let k = if let Some(p) = &state.property_list {
            // a. Let K be state.[[PropertyList]].
            p.clone()
        // 6. Else,
        } else {
            // a. Let K be ? EnumerableOwnPropertyNames(value, key).
            let keys = value.enumerable_own_property_names(PropertyNameKind::Key, context);
            // 11. Remove value from state.[[Stack]] on error.
            let keys = match keys {
                Ok(k) => k,
                Err(e) => {
                    let removed = state.stack_set.remove(value);
                    debug_assert!(removed);
                    return Err(e);
                }
            };
            // Unwrap is safe, because EnumerableOwnPropertyNames with kind "key" only returns string values.
            match keys
                .iter()
                .map(|v| {
                    v.to_string(context)
                        .js_expect("EnumerableOwnPropertyNames only returns strings")
                })
                .collect::<Result<Vec<_>, _>>()
            {
                Ok(k) => k,
                Err(e) => {
                    let removed = state.stack_set.remove(value);
                    debug_assert!(removed);
                    return Err(e.into());
                }
            }
        };

        // 7. Let partial be a new empty List.
        let mut partial = Vec::with_capacity(k.len());

        // 8. For each element P of K, do
        for p in &k {
            // a. Let strP be ? SerializeJSONProperty(state, P, value).
            let str_p = Self::serialize_json_property(state, p.clone(), value, context);
            let str_p = match str_p {
                Ok(v) => v,
                Err(e) => {
                    let removed = state.stack_set.remove(value);
                    debug_assert!(removed);
                    return Err(e);
                }
            };

            // b. If strP is not undefined, then
            if let Some(str_p) = str_p {
                // i. Let member be QuoteJSONString(P).
                let mut member = Self::quote_json_string(p).iter().collect::<Vec<_>>();

                // ii. Set member to the string-concatenation of member and ":".
                member.push(':' as u16);

                // iii. If state.[[Gap]] is not the empty String, then
                if !state.gap.is_empty() {
                    // 1. Set member to the string-concatenation of member and the code unit 0x0020 (SPACE).
                    member.push(' ' as u16);
                }

                // iv. Set member to the string-concatenation of member and strP.
                member.extend(str_p.iter());

                // v. Append member to partial.
                partial.push(member);
            }
        }

        // 9. If partial is empty, then
        let r#final = if partial.is_empty() {
            // a. Let final be "{}".
            js_string!("{}")
        // 10. Else,
        } else {
            // a. If state.[[Gap]] is the empty String, then
            if state.gap.is_empty() {
                // i. Let properties be the String value formed by concatenating all the element Strings of partial
                //    with each adjacent pair of Strings separated with the code unit 0x002C (COMMA).
                //    A comma is not inserted either before the first String or after the last String.
                // ii. Let final be the string-concatenation of "{", properties, and "}".
                let separator = utf16!(",");
                let result = once(utf16!("{"))
                    .chain(Itertools::intersperse(
                        partial.iter().map(Vec::as_slice),
                        separator,
                    ))
                    .chain(once(utf16!("}")))
                    .flatten()
                    .copied()
                    .collect::<Vec<_>>();
                js_string!(&result[..])
            // b. Else,
            } else {
                // i. Let separator be the string-concatenation of the code unit 0x002C (COMMA),
                //    the code unit 0x000A (LINE FEED), and state.[[Indent]].
                let mut separator = utf16!(",\n").to_vec();
                separator.extend(state.indent.iter());
                // ii. Let properties be the String value formed by concatenating all the element Strings of partial
                //     with each adjacent pair of Strings separated with separator.
                //     The separator String is not inserted either before the first String or after the last String.
                // iii. Let final be the string-concatenation of "{", the code
                //      unit 0x000A (LINE FEED), state.[[Indent]], properties,
                //      the code unit 0x000A (LINE FEED), stepback, and "}".
                let indent_vec = state.indent.to_vec();
                let stepback_vec = stepback.to_vec();
                let result = [utf16!("{\n"), &indent_vec[..]]
                    .into_iter()
                    .chain(Itertools::intersperse(
                        partial.iter().map(Vec::as_slice),
                        &separator,
                    ))
                    .chain([utf16!("\n"), &stepback_vec[..], utf16!("}")])
                    .flatten()
                    .copied()
                    .collect::<Vec<_>>();
                js_string!(&result[..])
            }
        };

        // 12. Set state.[[Indent]] to stepback.
        state.indent = stepback;

        // 11. Remove value from state.[[Stack]].
        let removed = state.stack_set.remove(value);
        debug_assert!(removed);

        // 13. Return final.
        Ok(r#final)
    }

    /// `25.5.2.5 SerializeJSONArray ( state, value )`
    ///
    /// More information:
    ///  - [ECMAScript reference][spec]
    ///
    /// [spec]: https://tc39.es/ecma262/#sec-serializejsonarray
    fn serialize_json_array(
        state: &mut StateRecord,
        value: &JsObject,
        context: &mut Context,
    ) -> JsResult<JsString> {
        // 1. If state.[[Stack]] contains value, throw a TypeError exception because the structure is cyclical.
        if !state.stack_set.insert(value.clone()) {
            return Err(JsNativeError::typ()
                .with_message("cyclic object value")
                .into());
        }

        // 3. Let stepback be state.[[Indent]].
        let stepback = state.indent.clone();

        // 4. Set state.[[Indent]] to the string-concatenation of state.[[Indent]] and state.[[Gap]].
        state.indent = js_string!(&state.indent, &state.gap);

        // 6. Let len be ? LengthOfArrayLike(value).
        let len = match value.length_of_array_like(context) {
            Ok(n) => n,
            Err(e) => {
                let removed = state.stack_set.remove(value);
                debug_assert!(removed);
                return Err(e);
            }
        };

        // 5. Let partial be a new empty List.
        let mut partial = Vec::with_capacity(len as usize);

        // 7. Let index be 0.
        let mut index = 0;

        // 8. Repeat, while index < len,
        while index < len {
            // a. Let strP be ? SerializeJSONProperty(state, ! ToString(𝔽(index)), value).
            let str_p = Self::serialize_json_property(state, index.into(), value, context);
            let str_p = match str_p {
                Ok(v) => v,
                Err(e) => {
                    let removed = state.stack_set.remove(value);
                    debug_assert!(removed);
                    return Err(e);
                }
            };

            // b. If strP is undefined, then
            if let Some(str_p) = str_p {
                // i. Append strP to partial.
                partial.push(Cow::Owned(str_p.iter().collect::<_>()));
            // c. Else,
            } else {
                // i. Append "null" to partial.
                partial.push(Cow::Borrowed(utf16!("null")));
            }

            // d. Set index to index + 1.
            index += 1;
        }

        // 9. If partial is empty, then
        let r#final = if partial.is_empty() {
            // a. Let final be "[]".
            js_string!("[]")
        // 10. Else,
        } else {
            // a. If state.[[Gap]] is the empty String, then
            if state.gap.is_empty() {
                // i. Let properties be the String value formed by concatenating all the element Strings of partial
                //    with each adjacent pair of Strings separated with the code unit 0x002C (COMMA).
                //    A comma is not inserted either before the first String or after the last String.
                // ii. Let final be the string-concatenation of "[", properties, and "]".
                let separator = utf16!(",");
                let result = once(utf16!("["))
                    .chain(Itertools::intersperse(
                        partial.iter().map(Cow::as_ref),
                        separator,
                    ))
                    .chain(once(utf16!("]")))
                    .flatten()
                    .copied()
                    .collect::<Vec<_>>();
                js_string!(&result[..])
            // b. Else,
            } else {
                // i. Let separator be the string-concatenation of the code unit 0x002C (COMMA),
                //    the code unit 0x000A (LINE FEED), and state.[[Indent]].
                let mut separator = utf16!(",\n").to_vec();
                separator.extend(state.indent.iter());
                // ii. Let properties be the String value formed by concatenating all the element Strings of partial
                //     with each adjacent pair of Strings separated with separator.
                //     The separator String is not inserted either before the first String or after the last String.
                // iii. Let final be the string-concatenation of "[", the code unit 0x000A (LINE FEED), state.[[Indent]], properties, the code unit 0x000A (LINE FEED), stepback, and "]".
                let indent_vec = state.indent.to_vec();
                let stepback_vec = stepback.to_vec();
                let result = [utf16!("[\n"), &indent_vec[..]]
                    .into_iter()
                    .chain(Itertools::intersperse(
                        partial.iter().map(Cow::as_ref),
                        &separator,
                    ))
                    .chain([utf16!("\n"), &stepback_vec[..], utf16!("]")])
                    .flatten()
                    .copied()
                    .collect::<Vec<_>>();
                js_string!(&result[..])
            }
        };

        // 12. Set state.[[Indent]] to stepback.
        state.indent = stepback;

        // 11. Remove value from state.[[Stack]].
        let removed = state.stack_set.remove(value);
        debug_assert!(removed);

        // 13. Return final.
        Ok(r#final)
    }
}

struct StateRecord {
    replacer_function: Option<JsObject>,
    stack_set: FxHashSet<JsObject>,
    indent: JsString,
    gap: JsString,
    property_list: Option<Vec<JsString>>,
}
