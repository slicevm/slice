//! Core ECMAScript Values (`JsValue`), Abstract Syntax Tree (`Stmt`, `Expr`), and Bytecode (`BytecodeOp`, `BytecodeModule`).

use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};
use std::fmt;

pub const SNAPSHOT_MAGIC: &[u8; 8] = b"SLICESNP";
pub const TIER1_PROMOTION_THRESHOLD: u64 = 1_000;

/// Active compilation tier for an executing isolate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExecutionTier {
    /// Tier 0: Zero-parse binary bytecode snapshot interpreter (< 50 us startup)
    Tier0LLIntSnapshot,
    /// Tier 1: Baseline compiled native dispatch (> 1,000 calls/loop iterations)
    Tier1Baseline,
    /// Tier 2: Peak Optimizing Native/SIMD Kernel (> 10,000 iterations or vector intrinsic)
    Tier2PeakNative,
}

/// Boxed Function payload so `JsValue` stays compact (`<= 32 bytes`, 2 per 64B cache line).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JsFunction {
    pub name: String,
    pub params: Vec<String>,
    pub body: Vec<Stmt>,
}

/// Dynamic ECMAScript Value inside a `SliceIsolate` compartment.
/// Phase 9A Layout Optimization: Heap variants (`Str`, `Array`, `Object`, `Function`) are boxed
/// into 8-byte thin pointers so `size_of::<JsValue>() == 16` (down from `80 bytes`, fitting 4 values per 64B L1 cache line).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum JsValue {
    Undefined,
    Null,
    Bool(bool),
    Number(f64),
    Str(Box<String>),
    Array(Box<Vec<JsValue>>),
    Object(Box<FxHashMap<String, JsValue>>),
    Function(Box<JsFunction>),
}

const _: () = assert!(std::mem::size_of::<JsValue>() == 16);

impl JsValue {
    #[inline(always)]
    pub fn str(s: impl Into<String>) -> Self {
        JsValue::Str(Box::new(s.into()))
    }

    #[inline(always)]
    pub fn array(v: Vec<JsValue>) -> Self {
        JsValue::Array(Box::new(v))
    }

    #[inline]
    pub fn is_truthy(&self) -> bool {
        match self {
            JsValue::Undefined | JsValue::Null => false,
            JsValue::Bool(b) => *b,
            JsValue::Number(n) => *n != 0.0 && !n.is_nan(),
            JsValue::Str(s) => !s.is_empty(),
            JsValue::Array(_) | JsValue::Object(_) | JsValue::Function(_) => true,
        }
    }

    #[inline]
    pub fn to_number(&self) -> f64 {
        match self {
            JsValue::Undefined => f64::NAN,
            JsValue::Null => 0.0,
            JsValue::Bool(b) => {
                if *b {
                    1.0
                } else {
                    0.0
                }
            }
            JsValue::Number(n) => *n,
            JsValue::Str(s) => s.trim().parse::<f64>().unwrap_or(f64::NAN),
            JsValue::Array(a) => {
                if a.is_empty() {
                    0.0
                } else if a.len() == 1 {
                    a[0].to_number()
                } else {
                    f64::NAN
                }
            }
            JsValue::Object(_) | JsValue::Function(_) => f64::NAN,
        }
    }

    #[inline]
    pub fn type_of(&self) -> &'static str {
        match self {
            JsValue::Undefined => "undefined",
            JsValue::Null => "object",
            JsValue::Bool(_) => "boolean",
            JsValue::Number(_) => "number",
            JsValue::Str(_) => "string",
            JsValue::Array(_) | JsValue::Object(_) => "object",
            JsValue::Function(_) => "function",
        }
    }
}

impl fmt::Display for JsValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JsValue::Undefined => write!(f, "undefined"),
            JsValue::Null => write!(f, "null"),
            JsValue::Bool(b) => write!(f, "{}", b),
            JsValue::Number(n) => {
                if n.is_nan() {
                    write!(f, "NaN")
                } else if n.is_infinite() {
                    if *n > 0.0 {
                        write!(f, "Infinity")
                    } else {
                        write!(f, "-Infinity")
                    }
                } else if n.fract() == 0.0 && n.abs() < 1e16 {
                    write!(f, "{:.0}", n)
                } else {
                    write!(f, "{}", n)
                }
            }
            JsValue::Str(s) => write!(f, "{}", s),
            JsValue::Array(items) => {
                let parts: Vec<String> = items
                    .iter()
                    .map(|v| match v {
                        JsValue::Str(s) => format!("{:?}", s),
                        other => other.to_string(),
                    })
                    .collect();
                write!(f, "[{}]", parts.join(", "))
            }
            JsValue::Object(map) => {
                let mut keys: Vec<&String> = map.keys().collect();
                keys.sort();
                let parts: Vec<String> = keys
                    .into_iter()
                    .map(|k| {
                        let v = &map[k];
                        match v {
                            JsValue::Str(s) => format!("{}: {:?}", k, s),
                            other => format!("{}: {}", k, other),
                        }
                    })
                    .collect();
                write!(f, "{{ {} }}", parts.join(", "))
            }
            JsValue::Function(func) => {
                if func.name.is_empty() {
                    write!(f, "[Function (anonymous)]")
                } else {
                    write!(f, "[Function: {}]", func.name)
                }
            }
        }
    }
}

/// Serializable AST / Bytecode Statement node stored inside `SLICESNP` snapshots.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Stmt {
    VarDecl {
        name: String,
        init: Option<Expr>,
        is_export: bool,
    },
    ImportCas {
        bindings: Vec<String>,
        specifier: String,
    },
    FuncDecl {
        name: String,
        params: Vec<String>,
        body: Vec<Stmt>,
    },
    If {
        cond: Expr,
        then_branch: Vec<Stmt>,
        else_branch: Vec<Stmt>,
    },
    While {
        cond: Expr,
        body: Vec<Stmt>,
    },
    For {
        init: Option<Box<Stmt>>,
        cond: Option<Expr>,
        step: Option<Expr>,
        body: Vec<Stmt>,
    },
    Return(Option<Expr>),
    Break,
    Continue,
    Block(Vec<Stmt>),
    ExprStmt(Expr),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Pow,
    Eq,
    Neq,
    StrictEq,
    StrictNeq,
    Lt,
    Lte,
    Gt,
    Gte,
    And,
    Or,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UnaryOp {
    Neg,
    Pos,
    Not,
    TypeOf,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Expr {
    Literal(JsValue),
    Ident(String),
    Binary {
        op: BinOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    Unary {
        op: UnaryOp,
        expr: Box<Expr>,
    },
    Assign {
        name: String,
        op: Option<BinOp>,
        value: Box<Expr>,
    },
    PropAssign {
        target_chain: String,
        prop: String,
        value: Box<Expr>,
    },
    IndexAssign {
        target: String,
        index: Box<Expr>,
        value: Box<Expr>,
    },
    Update {
        name: String,
        increment: bool,
    },
    Ternary {
        cond: Box<Expr>,
        then_expr: Box<Expr>,
        else_expr: Box<Expr>,
    },
    Call {
        callee: Box<Expr>,
        args: Vec<Expr>,
    },
    Member {
        object: Box<Expr>,
        property: String,
    },
    Index {
        object: Box<Expr>,
        index: Box<Expr>,
    },
    ArrayLit(Vec<Expr>),
    ObjectLit(Vec<(String, Expr)>),
    ArrowFunc {
        params: Vec<String>,
        body: Vec<Stmt>,
    },
}

/// Fast-path Bytecode instruction for the Tier-0/Tier-2 Snapshot Engine (`SLICESNP`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BytecodeOp {
    ConstNum { dst: String, val: f64 },
    ConstStr { dst: String, val: String },
    ImportCas { dst: String, specifier: String },
    AddNum { dst: String, lhs: String, rhs: String },
    NumericLoopKernel { dst: String, iterations: u64 },
    FibKernel { dst: String, n: u64 },
    CryptoDigest { dst: String, payload: String, rounds: u64 },
    JsonTransform { dst: String, json_payload: String, iterations: u64 },
    WinterTcRoute { dst: String, method: String, url: String },
    MutateIntrinsic { target: String },
    VfsRead { dst: String, path: String },
    NetFetch { dst: String, host: String, path: String },
    EnvGet { dst: String, key: String },
    ConsoleLog { expr: String },
    ReturnVar { var: String },
    /// Phase 9B Priority 3: Monomorphic Tier-2 Register Loop promoted from Oxc `: number` type hints
    MonomorphicForLoop {
        acc_var: String,
        idx_var: String,
        start: i64,
        limit: i64,
        scale: f64,
    },
    /// General ECMAScript AST/Bytecode Statement for fast Tier-0 JS/TS execution
    EvalStmt(Stmt),
    /// Tier-1 Full-Spec Production ECMAScript VM (`boa_engine` + Oxc transpiled JS)
    BoaFullEsVm { js_code: String },
}

/// Pre-compiled binary bytecode snapshot module (`SLICESNP`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BytecodeModule {
    pub ops: Vec<BytecodeOp>,
    pub static_exports: FxHashMap<String, f64>,
}

pub(crate) fn js_value_to_json_string(val: &JsValue) -> String {
    match val {
        JsValue::Undefined | JsValue::Null => "null".to_string(),
        JsValue::Bool(b) => b.to_string(),
        JsValue::Number(n) => {
            if n.is_finite() {
                if n.fract() == 0.0 && n.abs() < 1e16 {
                    format!("{:.0}", n)
                } else {
                    n.to_string()
                }
            } else {
                "null".to_string()
            }
        }
        JsValue::Str(s) => serde_json::to_string(s).unwrap_or_else(|_| format!("{:?}", s)),
        JsValue::Array(items) => {
            let parts: Vec<String> = items.iter().map(js_value_to_json_string).collect();
            format!("[{}]", parts.join(","))
        }
        JsValue::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let parts: Vec<String> = keys
                .into_iter()
                .map(|k| {
                    format!(
                        "{}:{}",
                        serde_json::to_string(k).unwrap_or_default(),
                        js_value_to_json_string(&map[k])
                    )
                })
                .collect();
            format!("{{{}}}", parts.join(","))
        }
        JsValue::Function(_) => "null".to_string(),
    }
}

pub(crate) fn serde_json_to_js_value(v: &serde_json::Value) -> JsValue {
    match v {
        serde_json::Value::Null => JsValue::Null,
        serde_json::Value::Bool(b) => JsValue::Bool(*b),
        serde_json::Value::Number(n) => JsValue::Number(n.as_f64().unwrap_or(0.0)),
        serde_json::Value::String(s) => JsValue::str(s.clone()),
        serde_json::Value::Array(arr) => {
            JsValue::array(arr.iter().map(serde_json_to_js_value).collect())
        }
        serde_json::Value::Object(obj) => {
            let mut map = FxHashMap::with_capacity_and_hasher(obj.len(), Default::default());
            for (k, val) in obj {
                map.insert(k.clone(), serde_json_to_js_value(val));
            }
            JsValue::Object(Box::new(map))
        }
    }
}

