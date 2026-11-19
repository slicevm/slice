// Slice (Nexus-JS) Lazy Prelude Shard 16: `structuredClone` (`16_structured_clone.js`)
(function() {
  var __defProp = Object.defineProperty;
  var __getOwnDesc = Object.getOwnPropertyDescriptor;
  var __getProto = Object.getPrototypeOf;
  var __hasOwn = Object.prototype.hasOwnProperty;

  var __dataDesc = Object.create(null);
  __dataDesc.value = undefined;
  __dataDesc.writable = true;
  __dataDesc.enumerable = true;
  __dataDesc.configurable = true;

  function __setClonedProp(target, key, val) {
    if (key === "__proto__") {
      __dataDesc.value = val;
      __defProp(target, key, __dataDesc);
      __dataDesc.value = undefined;
    } else {
      target[key] = val;
    }
  }

  function __getter(proto, prop, fallback) {
    var d = proto ? __getOwnDesc(proto, prop) : null;
    return (d && typeof d.get === "function") ? d.get : fallback;
  }

  var __AB = globalThis.ArrayBuffer;
  var __SAB = typeof SharedArrayBuffer === "function" ? SharedArrayBuffer : null;
  var __abDetached = __getter(__AB.prototype, "detached", function() { return false; });
  var __abByteLength = __getter(__AB.prototype, "byteLength", function() { return this.byteLength; });
  var __abResizable = __getter(__AB.prototype, "resizable", null);
  var __abMaxByteLength = __getter(__AB.prototype, "maxByteLength", function() { return this.byteLength; });
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

  var __taProto = __getProto(__U8.prototype);
  var __taBuffer = __getter(__taProto, "buffer", function() { return this.buffer; });
  var __taByteOffset = __getter(__taProto, "byteOffset", function() { return this.byteOffset; });
  var __taLength = __getter(__taProto, "length", function() { return this.length; });
  var __dvBuffer = __getter(__DV.prototype, "buffer", function() { return this.buffer; });
  var __dvByteOffset = __getter(__DV.prototype, "byteOffset", function() { return this.byteOffset; });
  var __dvByteLength = __getter(__DV.prototype, "byteLength", function() { return this.byteLength; });

  var __boolVal = Boolean.prototype.valueOf;
  var __numVal = Number.prototype.valueOf;
  var __strVal = String.prototype.valueOf;
  var __bigIntVal = typeof BigInt === "function" ? BigInt.prototype.valueOf : null;
  var __dateTime = Date.prototype.getTime;
  var __reSource = __getter(RegExp.prototype, "source", function() { return this.source; });
  var __reFlags = __getter(RegExp.prototype, "flags", function() { return this.flags; });

  var __WeakMap = typeof WeakMap === "function" ? WeakMap : null;
  var __WeakSet = typeof WeakSet === "function" ? WeakSet : null;
  var __WeakRef = typeof WeakRef === "function" ? WeakRef : null;
  var __FinReg = typeof FinalizationRegistry === "function" ? FinalizationRegistry : null;
  var __AggErr = typeof AggregateError === "function" ? AggregateError : null;

  function __throwCloneErr(msg) {
    if (!globalThis.DOMException) __slice_ensure_shard(13);
    throw new globalThis.DOMException(msg, "DataCloneError");
  }

  function __makeErrorInstance(nm, hasMsg, msg) {
    if (nm === "TypeError") return hasMsg ? new TypeError(msg) : new TypeError();
    if (nm === "RangeError") return hasMsg ? new RangeError(msg) : new RangeError();
    if (nm === "SyntaxError") return hasMsg ? new SyntaxError(msg) : new SyntaxError();
    if (nm === "ReferenceError") return hasMsg ? new ReferenceError(msg) : new ReferenceError();
    if (nm === "URIError") return hasMsg ? new URIError(msg) : new URIError();
    if (nm === "EvalError") return hasMsg ? new EvalError(msg) : new EvalError();
    return hasMsg ? new Error(msg) : new Error();
  }

  function __constructClonedView(val, clonedBuf, byteOff, lenOrByteLen, isAuto) {
    if (val instanceof __DV) return isAuto ? new __DV(clonedBuf, byteOff) : new __DV(clonedBuf, byteOff, lenOrByteLen);
    if (val instanceof __U8) return isAuto ? new __U8(clonedBuf, byteOff) : new __U8(clonedBuf, byteOff, lenOrByteLen);
    if (val instanceof __U8C) return isAuto ? new __U8C(clonedBuf, byteOff) : new __U8C(clonedBuf, byteOff, lenOrByteLen);
    if (val instanceof __I8) return isAuto ? new __I8(clonedBuf, byteOff) : new __I8(clonedBuf, byteOff, lenOrByteLen);
    if (val instanceof __U16) return isAuto ? new __U16(clonedBuf, byteOff) : new __U16(clonedBuf, byteOff, lenOrByteLen);
    if (val instanceof __I16) return isAuto ? new __I16(clonedBuf, byteOff) : new __I16(clonedBuf, byteOff, lenOrByteLen);
    if (val instanceof __U32) return isAuto ? new __U32(clonedBuf, byteOff) : new __U32(clonedBuf, byteOff, lenOrByteLen);
    if (val instanceof __I32) return isAuto ? new __I32(clonedBuf, byteOff) : new __I32(clonedBuf, byteOff, lenOrByteLen);
    if (val instanceof __F32) return isAuto ? new __F32(clonedBuf, byteOff) : new __F32(clonedBuf, byteOff, lenOrByteLen);
    if (val instanceof __F64) return isAuto ? new __F64(clonedBuf, byteOff) : new __F64(clonedBuf, byteOff, lenOrByteLen);
    if (__BI64 !== null && val instanceof __BI64) return isAuto ? new __BI64(clonedBuf, byteOff) : new __BI64(clonedBuf, byteOff, lenOrByteLen);
    if (__BU64 !== null && val instanceof __BU64) return isAuto ? new __BU64(clonedBuf, byteOff) : new __BU64(clonedBuf, byteOff, lenOrByteLen);
    __throwCloneErr("Unsupported ArrayBufferView cannot be cloned by structuredClone");
  }

  function __isNonCloneablePlatformObj(val) {
    if (
      val._isHeaders ||
      val._isSliceReq ||
      val._isSliceRes ||
      val._isFormData ||
      val._isUSP ||
      val._isSliceUrl
    ) {
      return true;
    }
    if (globalThis.ReadableStream && val instanceof globalThis.ReadableStream) return true;
    if (globalThis.WritableStream && val instanceof globalThis.WritableStream) return true;
    if (globalThis.TransformStream && val instanceof globalThis.TransformStream) return true;
    return false;
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
    var slotTag = __slice_sc_inspect(val);
    if (slotTag < 0) {
      if (slotTag === -1) {
        __throwCloneErr("DataCloneError: Cannot clone a detached or out-of-bounds ArrayBufferView");
      }
      if (slotTag === -2) {
        __throwCloneErr("DataCloneError: Object cannot be cloned by structuredClone");
      }
    } else if (slotTag > 0) {
      if (slotTag === 1 || slotTag === 2) {
        var isAuto = slotTag === 2;
        var isDv = val instanceof __DV;
        var rawBuf = isDv ? __dvBuffer.call(val) : __taBuffer.call(val);
        var byteOff = isDv ? __dvByteOffset.call(val) : __taByteOffset.call(val);
        var lenOrByteLen = isAuto ? 0 : (isDv ? __dvByteLength.call(val) : __taLength.call(val));
        var clonedBuf = __cloneDeep(rawBuf, memory);
        var viewCopy = __constructClonedView(val, clonedBuf, byteOff, lenOrByteLen, isAuto);
        memory.set(val, viewCopy);
        return viewCopy;
      }
      if (slotTag === 3) {
        var biCopy = Object(__bigIntVal.call(val));
        memory.set(val, biCopy);
        return biCopy;
      }
      if (slotTag === 4) {
        var boolCopy = new Boolean(__boolVal.call(val));
        memory.set(val, boolCopy);
        return boolCopy;
      }
      if (slotTag === 5) {
        var numCopy = new Number(__numVal.call(val));
        memory.set(val, numCopy);
        return numCopy;
      }
      if (slotTag === 6) {
        var strCopy = new String(__strVal.call(val));
        memory.set(val, strCopy);
        return strCopy;
      }
    }
    if (val instanceof __AB) {
      if (__abDetached.call(val)) {
        __throwCloneErr("DataCloneError: Cannot clone a detached ArrayBuffer");
      }
      var isRes = __abResizable !== null && __abResizable.call(val);
      var abCopy;
      if (!isRes && val.constructor === __AB) {
        abCopy = __abSlice.call(val, 0);
      } else {
        var abLen = __abByteLength.call(val);
        abCopy = isRes
          ? new __AB(abLen, { maxByteLength: __abMaxByteLength.call(val) })
          : new __AB(abLen);
        if (abLen > 0) {
          new __U8(abCopy).set(new __U8(val, 0, abLen));
        }
      }
      memory.set(val, abCopy);
      return abCopy;
    }
    if (__SAB !== null && val instanceof __SAB) {
      var sabCopy = __slice_sc_clone_sab(val);
      memory.set(val, sabCopy);
      return sabCopy;
    }
    if (val instanceof Date) {
      var dCopy = new Date(__dateTime.call(val));
      memory.set(val, dCopy);
      return dCopy;
    }
    if (val instanceof RegExp) {
      var rCopy = new RegExp(__reSource.call(val), __reFlags.call(val));
      memory.set(val, rCopy);
      return rCopy;
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
      (__FinReg !== null && val instanceof __FinReg) ||
      (globalThis.EventTarget && val instanceof globalThis.EventTarget) ||
      __isNonCloneablePlatformObj(val)
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
      var hasOwnMsg = __hasOwn.call(val, "message");
      var msgStr = hasOwnMsg && val.message !== undefined ? String(val.message) : "";
      var errName = val.name !== undefined ? String(val.name) : "Error";
      var eCopy;
      if (__AggErr !== null && val instanceof __AggErr) {
        eCopy = hasOwnMsg ? new __AggErr([], msgStr) : new __AggErr([]);
        memory.set(val, eCopy);
        if (Array.isArray(val.errors)) {
          eCopy.errors = __cloneDeep(val.errors, memory);
        }
      } else {
        eCopy = __makeErrorInstance(errName, hasOwnMsg, msgStr);
        memory.set(val, eCopy);
      }
      if (val.stack !== undefined) eCopy.stack = String(val.stack);
      if ("cause" in val) {
        __defProp(eCopy, "cause", {
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
      var aKeys = Object.keys(val);
      for (var j = 0; j < aKeys.length; j++) {
        var ak = aKeys[j];
        if (__hasOwn.call(val, ak)) {
          __setClonedProp(arrCopy, ak, __cloneDeep(val[ak], memory));
        }
      }
      return arrCopy;
    }
    var objCopy = {};
    memory.set(val, objCopy);
    var keys = Object.keys(val);
    for (var k = 0; k < keys.length; k++) {
      var prop = keys[k];
      if (__hasOwn.call(val, prop)) {
        __setClonedProp(objCopy, prop, __cloneDeep(val[prop], memory));
      }
    }
    return objCopy;
  }

  globalThis.structuredClone = function(value, options) {
    if (arguments.length === 0) throw new TypeError("structuredClone requires at least 1 argument");
    var transferSet = null;
    if (options !== undefined && options !== null) {
      if (typeof options !== "object" && typeof options !== "function") {
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
            if (__abDetached.call(t) || __slice_sc_inspect(t) === -3) {
              __throwCloneErr("DataCloneError: Cannot transfer an already detached or non-detachable ArrayBuffer");
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
