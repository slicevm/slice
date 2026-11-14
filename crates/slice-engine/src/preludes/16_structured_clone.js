// Slice (Nexus-JS) Lazy Prelude Shard 16: `structuredClone` (`16_structured_clone.js`)
(function() {
  var __AB = globalThis.ArrayBuffer;
  var __SAB = typeof SharedArrayBuffer === "function" ? SharedArrayBuffer : null;
  var __abDetachedDesc = Object.getOwnPropertyDescriptor(__AB.prototype, "detached");
  var __abDetached = __abDetachedDesc && typeof __abDetachedDesc.get === "function"
    ? __abDetachedDesc.get
    : function() { return false; };
  var __abSlice = __AB.prototype.slice;
  var __abTransfer = typeof __AB.prototype.transfer === "function" ? __AB.prototype.transfer : null;

  var __DV = globalThis.DataView;
  var __U8 = globalThis.Uint8Array;
  var __U8C = globalThis.Uint8ClampedArray;
  var __I8 = globalThis.Int8Array;
  var __U16 = globalThis.Uint16Array;
  var __I16 = globalThis.Int16Array;
  var __U32 = globalThis.Uint32Array;
  var __I32 = globalThis.Int32Array;
  var __F32 = globalThis.Float32Array;
  var __F64 = globalThis.Float64Array;
  var __BI64 = typeof BigInt64Array === "function" ? BigInt64Array : null;
  var __BU64 = typeof BigUint64Array === "function" ? BigUint64Array : null;

  var __WeakMap = typeof WeakMap === "function" ? WeakMap : null;
  var __WeakSet = typeof WeakSet === "function" ? WeakSet : null;
  var __WeakRef = typeof WeakRef === "function" ? WeakRef : null;
  var __AggErr = typeof AggregateError === "function" ? AggregateError : null;

  function __throwCloneErr(msg) {
    if (!globalThis.DOMException) __slice_ensure_shard(13);
    throw new globalThis.DOMException(msg, "DataCloneError");
  }

  function __makeErrorInstance(val, msg) {
    var nm = val.name;
    if (nm === "TypeError") return new TypeError(msg);
    if (nm === "RangeError") return new RangeError(msg);
    if (nm === "SyntaxError") return new SyntaxError(msg);
    if (nm === "ReferenceError") return new ReferenceError(msg);
    if (nm === "URIError") return new URIError(msg);
    if (nm === "EvalError") return new EvalError(msg);
    return new Error(msg);
  }

  function __constructClonedView(val, clonedBuf, byteOff, byteLen) {
    if (val instanceof __DV) return new __DV(clonedBuf, byteOff, byteLen);
    if (val instanceof __U8) return new __U8(clonedBuf, byteOff, val.length);
    if (val instanceof __U8C) return new __U8C(clonedBuf, byteOff, val.length);
    if (val instanceof __I8) return new __I8(clonedBuf, byteOff, val.length);
    if (val instanceof __U16) return new __U16(clonedBuf, byteOff, val.length);
    if (val instanceof __I16) return new __I16(clonedBuf, byteOff, val.length);
    if (val instanceof __U32) return new __U32(clonedBuf, byteOff, val.length);
    if (val instanceof __I32) return new __I32(clonedBuf, byteOff, val.length);
    if (val instanceof __F32) return new __F32(clonedBuf, byteOff, val.length);
    if (val instanceof __F64) return new __F64(clonedBuf, byteOff, val.length);
    if (__BI64 !== null && val instanceof __BI64) return new __BI64(clonedBuf, byteOff, val.length);
    if (__BU64 !== null && val instanceof __BU64) return new __BU64(clonedBuf, byteOff, val.length);
    __throwCloneErr("Unsupported ArrayBufferView cannot be cloned by structuredClone");
  }

  function __cloneDeep(val, memory) {
    if (val === null || typeof val !== "object") {
      if (typeof val === "function" || typeof val === "symbol") {
        __throwCloneErr("DataCloneError: Function or Symbol cannot be cloned by structuredClone");
      }
      return val;
    }
    if (memory.has(val)) {
      return memory.get(val);
    }
    if (val instanceof __AB) {
      if (__abDetached.call(val)) {
        __throwCloneErr("DataCloneError: Cannot clone a detached ArrayBuffer");
      }
      var abCopy = __abSlice.call(val, 0);
      memory.set(val, abCopy);
      return abCopy;
    }
    if (__SAB !== null && val instanceof __SAB) {
      memory.set(val, val);
      return val;
    }
    if (__AB.isView(val)) {
      var rawBuf = val.buffer;
      if (rawBuf instanceof __AB && __abDetached.call(rawBuf)) {
        __throwCloneErr("DataCloneError: Cannot clone a view backed by a detached ArrayBuffer");
      }
      var byteOff = val.byteOffset;
      var byteLen = val.byteLength;
      var clonedBuf = __cloneDeep(rawBuf, memory);
      var viewCopy = __constructClonedView(val, clonedBuf, byteOff, byteLen);
      memory.set(val, viewCopy);
      return viewCopy;
    }
    if (val instanceof Date) {
      var dCopy = new Date(val.getTime());
      memory.set(val, dCopy);
      return dCopy;
    }
    if (val instanceof RegExp) {
      var rCopy = new RegExp(val.source, val.flags);
      memory.set(val, rCopy);
      return rCopy;
    }
    if (val instanceof Boolean) {
      var boolCopy = new Boolean(val.valueOf());
      memory.set(val, boolCopy);
      return boolCopy;
    }
    if (val instanceof Number) {
      var numCopy = new Number(val.valueOf());
      memory.set(val, numCopy);
      return numCopy;
    }
    if (val instanceof String) {
      var strCopy = new String(val.valueOf());
      memory.set(val, strCopy);
      return strCopy;
    }
    if (val._isFile) {
      if (!globalThis.File) __slice_ensure_shard(15);
      var fCopy = new globalThis.File([new __U8(val._u8)], val.name, {
        type: val.type,
        lastModified: val.lastModified
      });
      memory.set(val, fCopy);
      return fCopy;
    }
    if (val._isBlob) {
      if (!globalThis.Blob) __slice_ensure_shard(15);
      var bCopy = new globalThis.Blob([new __U8(val._u8)], { type: val.type });
      memory.set(val, bCopy);
      return bCopy;
    }
    if (
      val instanceof Promise ||
      (__WeakMap !== null && val instanceof __WeakMap) ||
      (__WeakSet !== null && val instanceof __WeakSet) ||
      (__WeakRef !== null && val instanceof __WeakRef) ||
      (globalThis.EventTarget && val instanceof globalThis.EventTarget)
    ) {
      __throwCloneErr("DataCloneError: Object cannot be cloned by structuredClone");
    }
    if (val instanceof Map) {
      var mCopy = new Map();
      memory.set(val, mCopy);
      val.forEach(function(v, k) {
        mCopy.set(__cloneDeep(k, memory), __cloneDeep(v, memory));
      });
      return mCopy;
    }
    if (val instanceof Set) {
      var sCopy = new Set();
      memory.set(val, sCopy);
      val.forEach(function(v) {
        sCopy.add(__cloneDeep(v, memory));
      });
      return sCopy;
    }
    if (globalThis.DOMException && val instanceof globalThis.DOMException) {
      var deCopy = new globalThis.DOMException(val.message, val.name);
      if (val.stack) deCopy.stack = val.stack;
      memory.set(val, deCopy);
      return deCopy;
    }
    if (val instanceof Error) {
      var msgStr = val.message !== undefined ? String(val.message) : "";
      var eCopy;
      if (__AggErr !== null && val instanceof __AggErr) {
        eCopy = new __AggErr([], msgStr);
        memory.set(val, eCopy);
        if (Array.isArray(val.errors)) {
          eCopy.errors = __cloneDeep(val.errors, memory);
        }
      } else {
        eCopy = __makeErrorInstance(val, msgStr);
        if (val.name && eCopy.name !== val.name) eCopy.name = String(val.name);
        memory.set(val, eCopy);
      }
      if (val.stack !== undefined) eCopy.stack = val.stack;
      if ("cause" in val) {
        Object.defineProperty(eCopy, "cause", {
          value: __cloneDeep(val.cause, memory),
          writable: true,
          enumerable: false,
          configurable: true
        });
      }
      return eCopy;
    }
    if (Array.isArray(val)) {
      var arrCopy = new Array(val.length);
      memory.set(val, arrCopy);
      for (var j = 0; j < val.length; j++) {
        if (j in val) {
          arrCopy[j] = __cloneDeep(val[j], memory);
        }
      }
      return arrCopy;
    }
    var objCopy = {};
    memory.set(val, objCopy);
    var keys = Object.keys(val);
    for (var k = 0; k < keys.length; k++) {
      var prop = keys[k];
      objCopy[prop] = __cloneDeep(val[prop], memory);
    }
    return objCopy;
  }

  globalThis.structuredClone = function(value, options) {
    if (arguments.length === 0) throw new TypeError("structuredClone requires at least 1 argument");
    var transferSet = null;
    if (options !== undefined && options !== null) {
      if (typeof options !== "object") {
        throw new TypeError("structuredClone options must be an object");
      }
      if (options.transfer !== undefined && options.transfer !== null) {
        var list = options.transfer;
        if (!Array.isArray(list) && typeof list[Symbol.iterator] !== "function") {
          throw new TypeError("structuredClone options.transfer must be an iterable");
        }
        var items = Array.isArray(list) ? list : Array.from(list);
        if (items.length > 0) {
          transferSet = new Set();
          for (var i = 0; i < items.length; i++) {
            var t = items[i];
            if (!(t instanceof __AB)) {
              __throwCloneErr("DataCloneError: Transfer list item is not a transferable ArrayBuffer");
            }
            if (transferSet.has(t)) {
              __throwCloneErr("DataCloneError: Transfer list contains duplicate ArrayBuffer");
            }
            if (__abDetached.call(t)) {
              __throwCloneErr("DataCloneError: Cannot transfer an already detached ArrayBuffer");
            }
            transferSet.add(t);
          }
        }
      }
    }
    var memory = new Map();
    var result = __cloneDeep(value, memory);
    if (transferSet !== null && __abTransfer !== null) {
      transferSet.forEach(function(ab) {
        try {
          __abTransfer.call(ab, 0);
        } catch (_e) {
          __throwCloneErr("DataCloneError: ArrayBuffer cannot be detached");
        }
      });
    }
    return result;
  };
})();
