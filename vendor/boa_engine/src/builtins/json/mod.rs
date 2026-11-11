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

use std::{borrow::Cow, iter::once, rc::Rc};

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

/// Scans `bytes[pos..]` 8 bytes at a time for the first byte that is `< 0x20`, `b'"'`, or `b'\\'`.
#[inline]
fn scan_latin1_unescaped_json_bytes(bytes: &[u8], mut pos: usize) -> usize {
    const QUOTE_MASK: u64 = 0x2222_2222_2222_2222;
    const BS_MASK: u64 = 0x5C5C_5C5C_5C5C_5C5C;
    const LO_ONES: u64 = 0x0101_0101_0101_0101;
    const HI_BITS: u64 = 0x8080_8080_8080_8080;

    let len = bytes.len();
    while pos + 8 <= len {
        let chunk = u64::from_le_bytes(bytes[pos..pos + 8].try_into().unwrap());
        // Byte `b` is `>= 0x20` iff `(b & 0xE0) != 0` (at least one of bits 7, 6, 5 is set).
        let ge_20 = (chunk | (chunk << 1) | (chunk << 2)) & HI_BITS;
        let lt_20 = !ge_20 & HI_BITS;
        let xq = chunk ^ QUOTE_MASK;
        let eq_quote = xq.wrapping_sub(LO_ONES) & !xq & HI_BITS;
        let xb = chunk ^ BS_MASK;
        let eq_bs = xb.wrapping_sub(LO_ONES) & !xb & HI_BITS;
        let stop_mask = lt_20 | eq_quote | eq_bs;
        if stop_mask != 0 {
            return pos + ((stop_mask.trailing_zeros() >> 3) as usize);
        }
        pos += 8;
    }
    while pos < len {
        let b = bytes[pos];
        if b == b'"' || b == b'\\' || b < 0x20 {
            return pos;
        }
        pos += 1;
    }
    pos
}

/// Scans `bytes[pos..]` 8 bytes at a time for the first byte that requires JSON escaping or UTF-8
/// expansion (`< 0x20`, `b'"'`, `b'\\'`, or `>= 0x80`).
#[inline]
fn scan_ascii_unescaped_json_bytes(bytes: &[u8], mut pos: usize) -> usize {
    const QUOTE_MASK: u64 = 0x2222_2222_2222_2222;
    const BS_MASK: u64 = 0x5C5C_5C5C_5C5C_5C5C;
    const SPACE_MASK: u64 = 0x2020_2020_2020_2020;
    const LO_ONES: u64 = 0x0101_0101_0101_0101;
    const HI_BITS: u64 = 0x8080_8080_8080_8080;

    let len = bytes.len();
    while pos + 8 <= len {
        let chunk = u64::from_le_bytes(bytes[pos..pos + 8].try_into().unwrap());
        let non_ascii = chunk & HI_BITS;
        let lt_20 = chunk.wrapping_sub(SPACE_MASK) & !chunk & HI_BITS;
        let xq = chunk ^ QUOTE_MASK;
        let eq_quote = xq.wrapping_sub(LO_ONES) & !xq & HI_BITS;
        let xb = chunk ^ BS_MASK;
        let eq_bs = xb.wrapping_sub(LO_ONES) & !xb & HI_BITS;
        let stop_mask = non_ascii | lt_20 | eq_quote | eq_bs;
        if stop_mask != 0 {
            return pos + ((stop_mask.trailing_zeros() >> 3) as usize);
        }
        pos += 8;
    }
    while pos < len {
        let b = bytes[pos];
        if b < 0x20 || b == b'"' || b == b'\\' || b >= 0x80 {
            return pos;
        }
        pos += 1;
    }
    pos
}

trait JsonInput<'a>: Copy {
    fn len(self) -> usize;
    fn code_unit(self, idx: usize) -> u16;
    fn slice_to_js_string(self, start: usize, end: usize) -> JsString;
    fn latin1_slice(self, start: usize, end: usize) -> Option<&'a [u8]>;
    fn skip_ws(self, pos: usize) -> usize;
    fn scan_unescaped_string(self, pos: usize) -> usize;
    fn try_match_unescaped_key(self, pos: usize, expected: &JsString) -> Option<usize>;
    fn parse_f64(self, start: usize, end: usize) -> Option<f64>;
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
    fn latin1_slice(self, start: usize, end: usize) -> Option<&'a [u8]> {
        Some(&self[start..end])
    }

    #[inline]
    fn skip_ws(self, mut pos: usize) -> usize {
        let len = self.len();
        if pos < len && self[pos] > 0x20 {
            return pos;
        }
        while pos < len {
            match self[pos] {
                0x20 | 0x09 | 0x0A | 0x0D => pos += 1,
                _ => break,
            }
        }
        pos
    }

    #[inline]
    fn scan_unescaped_string(self, pos: usize) -> usize {
        scan_latin1_unescaped_json_bytes(self, pos)
    }

    #[inline]
    fn try_match_unescaped_key(self, pos: usize, expected: &JsString) -> Option<usize> {
        let boa_string::JsStrVariant::Latin1(exp_bytes) = expected.as_str().variant() else {
            return None;
        };
        let content_start = pos + 1;
        let content_end = content_start.checked_add(exp_bytes.len())?;
        if content_end < self.len()
            && self[content_end] == b'"'
            && &self[content_start..content_end] == exp_bytes
        {
            Some(content_end + 1)
        } else {
            None
        }
    }

    #[inline]
    fn parse_f64(self, start: usize, end: usize) -> Option<f64> {
        fast_float2::parse(&self[start..end]).ok()
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
    fn latin1_slice(self, _start: usize, _end: usize) -> Option<&'a [u8]> {
        None
    }

    #[inline]
    fn skip_ws(self, mut pos: usize) -> usize {
        let len = self.len();
        if pos < len && self[pos] > 0x20 {
            return pos;
        }
        while pos < len {
            match self[pos] {
                0x20 | 0x09 | 0x0A | 0x0D => pos += 1,
                _ => break,
            }
        }
        pos
    }

    #[inline]
    fn scan_unescaped_string(self, mut pos: usize) -> usize {
        let len = self.len();
        while pos + 4 <= len {
            let u0 = self[pos];
            let u1 = self[pos + 1];
            let u2 = self[pos + 2];
            let u3 = self[pos + 3];
            if u0 < 0x20
                || u0 == 0x22
                || u0 == 0x5C
                || u1 < 0x20
                || u1 == 0x22
                || u1 == 0x5C
                || u2 < 0x20
                || u2 == 0x22
                || u2 == 0x5C
                || u3 < 0x20
                || u3 == 0x22
                || u3 == 0x5C
            {
                break;
            }
            pos += 4;
        }
        while pos < len {
            let u = self[pos];
            if u == 0x22 || u == 0x5C || u < 0x20 {
                return pos;
            }
            pos += 1;
        }
        pos
    }

    #[inline]
    fn try_match_unescaped_key(self, pos: usize, expected: &JsString) -> Option<usize> {
        let content_start = pos + 1;
        match expected.as_str().variant() {
            boa_string::JsStrVariant::Latin1(exp_bytes) => {
                let content_end = content_start.checked_add(exp_bytes.len())?;
                if content_end < self.len()
                    && self[content_end] == 0x22
                    && self[content_start..content_end]
                        .iter()
                        .copied()
                        .eq(exp_bytes.iter().copied().map(u16::from))
                {
                    Some(content_end + 1)
                } else {
                    None
                }
            }
            boa_string::JsStrVariant::Utf16(exp_units) => {
                let content_end = content_start.checked_add(exp_units.len())?;
                if content_end < self.len()
                    && self[content_end] == 0x22
                    && &self[content_start..content_end] == exp_units
                {
                    Some(content_end + 1)
                } else {
                    None
                }
            }
        }
    }

    #[inline]
    fn parse_f64(self, start: usize, end: usize) -> Option<f64> {
        let slice = &self[start..end];
        if slice.len() <= 64 {
            let mut stack_buf = [0u8; 64];
            for (dst, &u) in stack_buf[..slice.len()].iter_mut().zip(slice.iter()) {
                *dst = u as u8;
            }
            fast_float2::parse(&stack_buf[..slice.len()]).ok()
        } else {
            let heap_buf: Vec<u8> = slice.iter().map(|&u| u as u8).collect();
            fast_float2::parse(&heap_buf).ok()
        }
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

trait SourceTracker {
    type Node;
    type ArrayNodes: Default;
    type ObjEntries: Default;

    fn primitive<'a, I: JsonInput<'a>>(
        input: I,
        start: usize,
        end: usize,
        val: &JsValue,
    ) -> Self::Node;
    fn literal_primitive(lit: JsString, val: &JsValue) -> Self::Node;
    fn push_array_node(nodes: &mut Self::ArrayNodes, node: Self::Node);
    fn finish_array(nodes: Self::ArrayNodes) -> Self::Node;
    fn push_obj_entry(entries: &mut Self::ObjEntries, key: &JsString, node: Self::Node);
    fn finish_object(entries: Self::ObjEntries) -> Self::Node;
}

struct NoSource;

impl SourceTracker for NoSource {
    type Node = ();
    type ArrayNodes = ();
    type ObjEntries = ();

    #[inline(always)]
    fn primitive<'a, I: JsonInput<'a>>(
        _input: I,
        _start: usize,
        _end: usize,
        _val: &JsValue,
    ) -> Self::Node {
    }

    #[inline(always)]
    fn literal_primitive(_lit: JsString, _val: &JsValue) -> Self::Node {}

    #[inline(always)]
    fn push_array_node(_nodes: &mut Self::ArrayNodes, _node: Self::Node) {}

    #[inline(always)]
    fn finish_array(_nodes: Self::ArrayNodes) -> Self::Node {}

    #[inline(always)]
    fn push_obj_entry(_entries: &mut Self::ObjEntries, _key: &JsString, _node: Self::Node) {}

    #[inline(always)]
    fn finish_object(_entries: Self::ObjEntries) -> Self::Node {}
}

struct WithSource;

impl SourceTracker for WithSource {
    type Node = JsonNode;
    type ArrayNodes = Vec<JsonNode>;
    type ObjEntries = Vec<(JsString, JsonNode)>;

    #[inline]
    fn primitive<'a, I: JsonInput<'a>>(
        input: I,
        start: usize,
        end: usize,
        val: &JsValue,
    ) -> Self::Node {
        JsonNode::Primitive(input.slice_to_js_string(start, end), val.clone())
    }

    #[inline]
    fn literal_primitive(lit: JsString, val: &JsValue) -> Self::Node {
        JsonNode::Primitive(lit, val.clone())
    }

    #[inline]
    fn push_array_node(nodes: &mut Self::ArrayNodes, node: Self::Node) {
        nodes.push(node);
    }

    #[inline]
    fn finish_array(nodes: Self::ArrayNodes) -> Self::Node {
        JsonNode::Array(nodes)
    }

    #[inline]
    fn push_obj_entry(entries: &mut Self::ObjEntries, key: &JsString, node: Self::Node) {
        entries.push((key.clone(), node));
    }

    #[inline]
    fn finish_object(entries: Self::ObjEntries) -> Self::Node {
        JsonNode::Object(entries)
    }
}

#[inline]
fn is_cacheable_chain_key(key: &JsString) -> bool {
    match key.as_str().variant() {
        boa_string::JsStrVariant::Latin1(bytes) => {
            bytes.iter().all(|&b| b >= 0x20 && b != b'"' && b != b'\\')
        }
        boa_string::JsStrVariant::Utf16(units) => {
            units.iter().all(|&u| u >= 0x20 && u != 0x22 && u != 0x5C)
        }
    }
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
                    let (val, node) = Self::parse_ecma404::<_, WithSource>(bytes, context)?;
                    (val, Some(node))
                } else {
                    let (val, ()) = Self::parse_ecma404::<_, NoSource>(bytes, context)?;
                    (val, None)
                }
            }
            boa_string::JsStrVariant::Utf16(units) => {
                if reviver_obj.is_some() {
                    let (val, node) = Self::parse_ecma404::<_, WithSource>(units, context)?;
                    (val, Some(node))
                } else {
                    let (val, ()) = Self::parse_ecma404::<_, NoSource>(units, context)?;
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

    /// Direct ECMA-404 iterative JSON parser over `Latin1(&[u8])` or `Utf16(&[u16])`.
    ///
    /// - Uses `fast-float2` for exact IEEE-754 round-to-nearest-even `f64` conversion and `+-Infinity` on overflow.
    /// - Preserves lone UTF-16 surrogates (`\uD800`..`\uDFFF` and literal unpaired surrogates).
    /// - Zero-copy `JsStr::latin1` / `utf16` slicing with 8-byte SWAR string scanning.
    /// - Uses `SharedShape` and per-depth shape-chain prediction for ordinary objects so homogeneous object arrays skip key allocation, hashing, and shape transitions.
    /// - Uses an explicit heap-allocated container stack (`O(1)` C/Rust call stack) supporting up to 2048 nesting levels.
    fn parse_ecma404<'a, I: JsonInput<'a>, S: SourceTracker>(
        input: I,
        context: &mut Context,
    ) -> JsResult<(JsValue, S::Node)> {
        use crate::builtins::OrdinaryObject;
        use crate::object::{IndexedProperties, PropertyMap, shape::SharedShape};
        use crate::property::PropertyKey;

        const MAX_DEPTH: usize = 2048;
        const SHAPE_CACHE_DEPTHS: usize = 16;
        const MAX_CHAIN_KEYS: usize = 64;

        type ShapeChain = Rc<[(JsString, SharedShape)]>;

        enum ContainerFrame<S: SourceTracker> {
            Array {
                elements: thin_vec::ThinVec<JsValue>,
                child_nodes: S::ArrayNodes,
            },
            ObjectFast {
                chain: ShapeChain,
                matched_len: usize,
                storage: Vec<JsValue>,
                entries: S::ObjEntries,
                depth: usize,
            },
            ObjectGeneric {
                props: PropertyMap,
                chain_builder: Option<Vec<(JsString, SharedShape)>>,
                current_key: JsString,
                entries: S::ObjEntries,
                depth: usize,
            },
        }

        struct ParserState<'a, I> {
            input: I,
            pos: usize,
            depth_chains: [Option<ShapeChain>; SHAPE_CACHE_DEPTHS],
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
                self.pos = self.input.skip_ws(self.pos);
            }

            fn parse_root<St: SourceTracker>(
                &mut self,
                context: &mut Context,
            ) -> JsResult<(JsValue, St::Node)> {
                let mut stack: Vec<ContainerFrame<St>> = Vec::new();
                let base_obj_shape = context
                    .intrinsics()
                    .templates()
                    .ordinary_object()
                    .shape()
                    .clone();

                'parse_val: loop {
                    self.skip_ws();
                    if self.pos >= self.input.len() {
                        return Err(self.syntax_err("unexpected end of JSON input"));
                    }
                    let (mut cur_val, mut cur_node) = match self.input.code_unit(self.pos) {
                        0x22 => {
                            let start = self.pos;
                            let s = self.parse_string()?;
                            let val = JsValue::new(s);
                            let node = St::primitive(self.input, start, self.pos, &val);
                            (val, node)
                        }
                        0x5B => {
                            if stack.len() >= MAX_DEPTH {
                                return Err(self.syntax_err("JSON nesting depth exceeded"));
                            }
                            self.pos += 1;
                            self.skip_ws();
                            if self.pos < self.input.len() && self.input.code_unit(self.pos) == 0x5D
                            {
                                self.pos += 1;
                                let arr = crate::builtins::Array::array_create(0, None, context)?;
                                let node = St::finish_array(St::ArrayNodes::default());
                                (arr.into(), node)
                            } else {
                                stack.push(ContainerFrame::Array {
                                    elements: thin_vec::ThinVec::new(),
                                    child_nodes: St::ArrayNodes::default(),
                                });
                                continue 'parse_val;
                            }
                        }
                        0x7B => {
                            let depth = stack.len();
                            if depth >= MAX_DEPTH {
                                return Err(self.syntax_err("JSON nesting depth exceeded"));
                            }
                            self.pos += 1;
                            self.skip_ws();
                            if self.pos < self.input.len() && self.input.code_unit(self.pos) == 0x7D
                            {
                                self.pos += 1;
                                let obj = context
                                    .intrinsics()
                                    .templates()
                                    .ordinary_object()
                                    .create(OrdinaryObject, Vec::new());
                                let node = St::finish_object(St::ObjEntries::default());
                                (obj.into(), node)
                            } else {
                                if self.pos >= self.input.len()
                                    || self.input.code_unit(self.pos) != 0x22
                                {
                                    return Err(
                                        self.syntax_err("expected string key in JSON object")
                                    );
                                }
                                if depth < SHAPE_CACHE_DEPTHS
                                    && let Some(ref chain) = self.depth_chains[depth]
                                    && let Some((exp_key, _)) = chain.first()
                                    && let Some(after_quote) =
                                        self.input.try_match_unescaped_key(self.pos, exp_key)
                                {
                                    let chain_cloned = chain.clone();
                                    let cap = chain_cloned.len();
                                    self.pos = after_quote;
                                    self.skip_ws();
                                    if self.pos >= self.input.len()
                                        || self.input.code_unit(self.pos) != 0x3A
                                    {
                                        return Err(self.syntax_err(
                                            "expected ':' after key in JSON object",
                                        ));
                                    }
                                    self.pos += 1;
                                    stack.push(ContainerFrame::ObjectFast {
                                        chain: chain_cloned,
                                        matched_len: 0,
                                        storage: Vec::with_capacity(cap),
                                        entries: St::ObjEntries::default(),
                                        depth,
                                    });
                                    continue 'parse_val;
                                }

                                let key = self.parse_string()?;
                                self.skip_ws();
                                if self.pos >= self.input.len()
                                    || self.input.code_unit(self.pos) != 0x3A
                                {
                                    return Err(
                                        self.syntax_err("expected ':' after key in JSON object")
                                    );
                                }
                                self.pos += 1;
                                let props = PropertyMap::from_parts(
                                    base_obj_shape.clone().into(),
                                    Vec::with_capacity(4),
                                    IndexedProperties::default(),
                                );
                                let chain_builder = (depth < SHAPE_CACHE_DEPTHS)
                                    .then(|| Vec::with_capacity(8));
                                stack.push(ContainerFrame::ObjectGeneric {
                                    props,
                                    chain_builder,
                                    current_key: key,
                                    entries: St::ObjEntries::default(),
                                    depth,
                                });
                                continue 'parse_val;
                            }
                        }
                        0x74 => {
                            if self.pos + 4 <= self.input.len()
                                && self.input.code_unit(self.pos + 1) == 0x72
                                && self.input.code_unit(self.pos + 2) == 0x75
                                && self.input.code_unit(self.pos + 3) == 0x65
                            {
                                self.pos += 4;
                                let val = JsValue::new(true);
                                let node = St::literal_primitive(js_string!("true"), &val);
                                (val, node)
                            } else {
                                return Err(self.syntax_err("expected 'true'"));
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
                                let node = St::literal_primitive(js_string!("false"), &val);
                                (val, node)
                            } else {
                                return Err(self.syntax_err("expected 'false'"));
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
                                let node = St::literal_primitive(js_string!("null"), &val);
                                (val, node)
                            } else {
                                return Err(self.syntax_err("expected 'null'"));
                            }
                        }
                        0x2D | 0x30..=0x39 => {
                            let start = self.pos;
                            let val = self.parse_number()?;
                            let node = St::primitive(self.input, start, self.pos, &val);
                            (val, node)
                        }
                        _ => return Err(self.syntax_err("unexpected token in JSON")),
                    };

                    'attach: loop {
                        let Some(top) = stack.last_mut() else {
                            return Ok((cur_val, cur_node));
                        };
                        match top {
                            ContainerFrame::Array {
                                elements,
                                child_nodes,
                            } => {
                                elements.push(cur_val);
                                St::push_array_node(child_nodes, cur_node);
                                self.skip_ws();
                                if self.pos >= self.input.len() {
                                    return Err(self.syntax_err("unterminated array in JSON"));
                                }
                                match self.input.code_unit(self.pos) {
                                    0x2C => {
                                        self.pos += 1;
                                        self.skip_ws();
                                        if self.pos < self.input.len()
                                            && self.input.code_unit(self.pos) == 0x5D
                                        {
                                            return Err(
                                                self.syntax_err("trailing comma in JSON array")
                                            );
                                        }
                                        continue 'parse_val;
                                    }
                                    0x5D => {
                                        self.pos += 1;
                                        let Some(ContainerFrame::Array {
                                            elements,
                                            child_nodes,
                                        }) = stack.pop()
                                        else {
                                            unreachable!()
                                        };
                                        let arr = crate::builtins::Array::array_create(
                                            elements.len() as u64,
                                            None,
                                            context,
                                        )?;
                                        arr.borrow_mut()
                                            .properties_mut()
                                            .override_indexed_properties(elements);
                                        cur_val = arr.into();
                                        cur_node = St::finish_array(child_nodes);
                                        continue 'attach;
                                    }
                                    _ => {
                                        return Err(
                                            self.syntax_err("expected ',' or ']' in JSON array")
                                        );
                                    }
                                }
                            }
                            ContainerFrame::ObjectFast {
                                chain,
                                matched_len,
                                storage,
                                entries,
                                depth,
                            } => {
                                St::push_obj_entry(entries, &chain[*matched_len].0, cur_node);
                                storage.push(cur_val);
                                *matched_len += 1;
                                self.skip_ws();
                                if self.pos >= self.input.len() {
                                    return Err(self.syntax_err("unterminated object in JSON"));
                                }
                                match self.input.code_unit(self.pos) {
                                    0x2C => {
                                        self.pos += 1;
                                        self.skip_ws();
                                        if self.pos < self.input.len()
                                            && self.input.code_unit(self.pos) == 0x7D
                                        {
                                            return Err(
                                                self.syntax_err("trailing comma in JSON object")
                                            );
                                        }
                                        if self.pos >= self.input.len()
                                            || self.input.code_unit(self.pos) != 0x22
                                        {
                                            return Err(self
                                                .syntax_err("expected string key in JSON object"));
                                        }
                                        if *matched_len < chain.len()
                                            && let Some(after_quote) = self
                                                .input
                                                .try_match_unescaped_key(
                                                    self.pos,
                                                    &chain[*matched_len].0,
                                                )
                                        {
                                            self.pos = after_quote;
                                            self.skip_ws();
                                            if self.pos >= self.input.len()
                                                || self.input.code_unit(self.pos) != 0x3A
                                            {
                                                return Err(self.syntax_err(
                                                    "expected ':' after key in JSON object",
                                                ));
                                            }
                                            self.pos += 1;
                                            continue 'parse_val;
                                        }

                                        let next_key = self.parse_string()?;
                                        self.skip_ws();
                                        if self.pos >= self.input.len()
                                            || self.input.code_unit(self.pos) != 0x3A
                                        {
                                            return Err(self.syntax_err(
                                                "expected ':' after key in JSON object",
                                            ));
                                        }
                                        self.pos += 1;
                                        let matched = *matched_len;
                                        let frame_depth = *depth;
                                        let prev_shape = chain[matched - 1].1.clone();
                                        let chain_builder = (frame_depth < SHAPE_CACHE_DEPTHS
                                            && matched < MAX_CHAIN_KEYS)
                                            .then(|| chain[..matched].to_vec());
                                        let props = PropertyMap::from_parts(
                                            prev_shape.into(),
                                            std::mem::take(storage),
                                            IndexedProperties::default(),
                                        );
                                        let entries_taken = std::mem::take(entries);
                                        *top = ContainerFrame::ObjectGeneric {
                                            props,
                                            chain_builder,
                                            current_key: next_key,
                                            entries: entries_taken,
                                            depth: frame_depth,
                                        };
                                        continue 'parse_val;
                                    }
                                    0x7D => {
                                        self.pos += 1;
                                        let Some(ContainerFrame::ObjectFast {
                                            chain,
                                            matched_len,
                                            storage,
                                            entries,
                                            ..
                                        }) = stack.pop()
                                        else {
                                            unreachable!()
                                        };
                                        let final_shape = chain[matched_len - 1].1.clone();
                                        let obj = JsObject::from_property_map(
                                            PropertyMap::from_parts(
                                                final_shape.into(),
                                                storage,
                                                IndexedProperties::default(),
                                            ),
                                        );
                                        cur_val = obj.into();
                                        cur_node = St::finish_object(entries);
                                        continue 'attach;
                                    }
                                    _ => {
                                        return Err(
                                            self.syntax_err("expected ',' or '}' in JSON object")
                                        );
                                    }
                                }
                            }
                            ContainerFrame::ObjectGeneric {
                                props,
                                chain_builder,
                                current_key,
                                entries,
                                ..
                            } => {
                                let key = std::mem::take(current_key);
                                St::push_obj_entry(entries, &key, cur_node);
                                let prev_storage_len = props.storage.len();
                                let pk = PropertyKey::from(key);
                                let key_for_chain = if chain_builder.is_some() {
                                    match &pk {
                                        PropertyKey::String(s)
                                            if prev_storage_len < MAX_CHAIN_KEYS
                                                && is_cacheable_chain_key(s) =>
                                        {
                                            Some(s.clone())
                                        }
                                        _ => None,
                                    }
                                } else {
                                    None
                                };
                                props.insert_data_property(pk, cur_val);
                                if let Some(builder) = chain_builder {
                                    if let Some(chain_key) = key_for_chain
                                        && props.storage.len() == prev_storage_len + 1
                                        && let Some(s) = props.shape.as_shared()
                                    {
                                        builder.push((chain_key, s.clone()));
                                    } else {
                                        *chain_builder = None;
                                    }
                                }
                                self.skip_ws();
                                if self.pos >= self.input.len() {
                                    return Err(self.syntax_err("unterminated object in JSON"));
                                }
                                match self.input.code_unit(self.pos) {
                                    0x2C => {
                                        self.pos += 1;
                                        self.skip_ws();
                                        if self.pos < self.input.len()
                                            && self.input.code_unit(self.pos) == 0x7D
                                        {
                                            return Err(
                                                self.syntax_err("trailing comma in JSON object")
                                            );
                                        }
                                        if self.pos >= self.input.len()
                                            || self.input.code_unit(self.pos) != 0x22
                                        {
                                            return Err(self
                                                .syntax_err("expected string key in JSON object"));
                                        }
                                        let next_key = self.parse_string()?;
                                        self.skip_ws();
                                        if self.pos >= self.input.len()
                                            || self.input.code_unit(self.pos) != 0x3A
                                        {
                                            return Err(self.syntax_err(
                                                "expected ':' after key in JSON object",
                                            ));
                                        }
                                        self.pos += 1;
                                        *current_key = next_key;
                                        continue 'parse_val;
                                    }
                                    0x7D => {
                                        self.pos += 1;
                                        let Some(ContainerFrame::ObjectGeneric {
                                            props,
                                            chain_builder,
                                            entries,
                                            depth,
                                            ..
                                        }) = stack.pop()
                                        else {
                                            unreachable!()
                                        };
                                        if depth < SHAPE_CACHE_DEPTHS
                                            && let Some(built) = chain_builder
                                            && !built.is_empty()
                                        {
                                            self.depth_chains[depth] = Some(built.into());
                                        }
                                        let obj = JsObject::from_property_map(props);
                                        cur_val = obj.into();
                                        cur_node = St::finish_object(entries);
                                        continue 'attach;
                                    }
                                    _ => {
                                        return Err(
                                            self.syntax_err("expected ',' or '}' in JSON object")
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            }

            #[inline]
            fn parse_string(&mut self) -> JsResult<JsString> {
                debug_assert_eq!(self.input.code_unit(self.pos), 0x22);
                self.pos += 1;
                let content_start = self.pos;
                let stop = self.input.scan_unescaped_string(content_start);
                self.pos = stop;
                if stop < self.input.len() {
                    let c = self.input.code_unit(stop);
                    if c == 0x22 {
                        let s = self.input.slice_to_js_string(content_start, stop);
                        self.pos = stop + 1;
                        return Ok(s);
                    }
                    if c == 0x5C {
                        return self.parse_string_escaped(content_start);
                    }
                    return Err(self.syntax_err("unescaped control character in JSON string"));
                }
                Err(self.syntax_err("unterminated string in JSON"))
            }

            fn parse_string_escaped(&mut self, content_start: usize) -> JsResult<JsString> {
                let len = self.input.len();
                if let Some(prefix_bytes) = self.input.latin1_slice(content_start, self.pos) {
                    let mut latin1 = Vec::with_capacity(prefix_bytes.len() + 16);
                    latin1.extend_from_slice(prefix_bytes);

                    while self.pos < len {
                        let c = self.input.code_unit(self.pos);
                        if c == 0x22 {
                            self.pos += 1;
                            return Ok(JsString::from(boa_string::JsStr::latin1(&latin1)));
                        }
                        if c < 0x20 {
                            return Err(
                                self.syntax_err("unescaped control character in JSON string")
                            );
                        }
                        if c == 0x5C {
                            self.pos += 1;
                            if self.pos >= len {
                                return Err(
                                    self.syntax_err("unterminated escape sequence in JSON")
                                );
                            }
                            let esc = self.input.code_unit(self.pos);
                            self.pos += 1;
                            match esc {
                                0x22 => latin1.push(0x22),
                                0x5C => latin1.push(0x5C),
                                0x2F => latin1.push(0x2F),
                                0x62 => latin1.push(0x08),
                                0x66 => latin1.push(0x0C),
                                0x6E => latin1.push(0x0A),
                                0x72 => latin1.push(0x0D),
                                0x74 => latin1.push(0x09),
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
                                                return Err(self.syntax_err(
                                                    "invalid unicode hex digit in JSON",
                                                ));
                                            }
                                        };
                                        code = (code << 4) | nibble;
                                    }
                                    if code <= 0xFF {
                                        latin1.push(code as u8);
                                    } else {
                                        let mut units: Vec<u16> =
                                            latin1.iter().map(|&b| u16::from(b)).collect();
                                        units.push(code);
                                        return self.parse_string_escaped_utf16_tail(units);
                                    }
                                }
                                _ => {
                                    return Err(
                                        self.syntax_err("invalid escape character in JSON")
                                    );
                                }
                            }
                        } else {
                            let chunk_start = self.pos;
                            self.pos = self.input.scan_unescaped_string(chunk_start + 1);
                            if let Some(chunk) = self.input.latin1_slice(chunk_start, self.pos) {
                                latin1.extend_from_slice(chunk);
                            }
                        }
                    }
                    return Err(self.syntax_err("unterminated string in JSON"));
                }

                let mut units = Vec::with_capacity((self.pos - content_start) + 16);
                for idx in content_start..self.pos {
                    units.push(self.input.code_unit(idx));
                }
                self.parse_string_escaped_utf16_tail(units)
            }

            fn parse_string_escaped_utf16_tail(&mut self, mut units: Vec<u16>) -> JsResult<JsString> {
                let len = self.input.len();
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

            #[inline]
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
                let mut acc: u64 = 0;
                let mut digits: usize;
                let first = self.input.code_unit(self.pos);
                match first {
                    0x30 => {
                        self.pos += 1;
                        digits = 1;
                        if self.pos < len && matches!(self.input.code_unit(self.pos), 0x30..=0x39) {
                            return Err(self.syntax_err("leading zero in JSON number"));
                        }
                    }
                    0x31..=0x39 => {
                        acc = u64::from(first - 0x30);
                        digits = 1;
                        self.pos += 1;
                        while self.pos < len {
                            let d = self.input.code_unit(self.pos);
                            if !matches!(d, 0x30..=0x39) {
                                break;
                            }
                            if digits < 19 {
                                acc = acc * 10 + u64::from(d - 0x30);
                            }
                            digits += 1;
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
                    self.pos += 1;
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
                    self.pos += 1;
                    while self.pos < len && matches!(self.input.code_unit(self.pos), 0x30..=0x39) {
                        self.pos += 1;
                    }
                }

                if !is_float {
                    if is_neg && acc == 0 {
                        return Ok(JsValue::new(-0.0f64));
                    }
                    if digits <= 16 && acc <= 9_007_199_254_740_992 {
                        if is_neg {
                            if acc <= (i32::MAX as u64) + 1 {
                                return Ok(JsValue::new((-(acc as i64)) as i32));
                            }
                            return Ok(JsValue::new(-(acc as f64)));
                        }
                        if acc <= i32::MAX as u64 {
                            return Ok(JsValue::new(acc as i32));
                        }
                        return Ok(JsValue::new(acc as f64));
                    }
                }

                let f = self
                    .input
                    .parse_f64(start, self.pos)
                    .ok_or_else(|| self.syntax_err("invalid number in JSON"))?;
                Ok(JsValue::new(f))
            }
        }

        let mut state = ParserState {
            input,
            pos: 0,
            depth_chains: std::array::from_fn(|_| None),
            _marker: std::marker::PhantomData,
        };
        let res = state.parse_root::<S>(context)?;
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
                Self::parse_ecma404::<_, NoSource>(bytes, context)?;
            }
            boa_string::JsStrVariant::Utf16(units) => {
                Self::parse_ecma404::<_, NoSource>(units, context)?;
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
                    let len = bytes.len();
                    while start < len {
                        let i = scan_ascii_unescaped_json_bytes(bytes, start);
                        if i > start {
                            buf.extend_from_slice(&bytes[start..i]);
                        }
                        if i >= len {
                            break;
                        }
                        let b = bytes[i];
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
                    buf.push(b'"');
                    true
                }
                boa_string::JsStrVariant::Utf16(units) => {
                    buf.push(b'"');
                    let mut utf8_tmp = [0u8; 4];
                    for decoded in char::decode_utf16(units.iter().copied()) {
                        let c = match decoded {
                            Ok(c) => c,
                            Err(unpaired) => {
                                let u = unpaired.unpaired_surrogate();
                                buf.extend_from_slice(&[
                                    b'\\',
                                    b'u',
                                    HEX[((u >> 12) & 0xF) as usize],
                                    HEX[((u >> 8) & 0xF) as usize],
                                    HEX[((u >> 4) & 0xF) as usize],
                                    HEX[(u & 0xF) as usize],
                                ]);
                                continue;
                            }
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
            verified_obj_shape_addr: &mut usize,
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
                    let shape_addr = props.shape.to_addr_usize();
                    let shape_preverified =
                        shape_addr != 0 && shape_addr == *verified_obj_shape_addr;
                    if !shape_preverified && props.shape.lookup(to_json_key).is_some() {
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
                        if shape_addr != arr_template_shape_addr {
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
                                        verified_obj_shape_addr,
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
                    if !shape_preverified {
                        if let Some(ref p) = props.shape.prototype() {
                            if p != obj_proto {
                                return Ok(FastStatus::Fallback);
                            }
                        }
                        *verified_obj_shape_addr = shape_addr;
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
                                    verified_obj_shape_addr,
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
        let mut verified_obj_shape_addr = 0usize;

        match write_val(
            value,
            &mut buf,
            &mut all_ascii,
            &mut stack,
            &obj_proto,
            &arr_proto,
            arr_template_shape_addr,
            &mut verified_obj_shape_addr,
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
