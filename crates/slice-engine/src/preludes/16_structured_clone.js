// Slice (Nexus-JS) Lazy Prelude Shard 16: `structuredClone` (`16_structured_clone.js`)
(function() {
  function __cloneDeep(val, seenSrc, seenDst) {
    if (val === null || typeof val !== "object") {
      if (typeof val === "function" || typeof val === "symbol") {
        if (!globalThis.DOMException) __slice_ensure_shard(13);
        throw new DOMException("DataCloneError: Function or Symbol cannot be cloned by structuredClone", "DataCloneError");
      }
      return val;
    }
    for (var i = 0; i < seenSrc.length; i++) {
      if (seenSrc[i] === val) return seenDst[i];
    }
    if (val instanceof Date) return new Date(val.getTime());
    if (val instanceof RegExp) return new RegExp(val.source, val.flags);
    if (val instanceof ArrayBuffer) {
      var abCopy = val.slice(0);
      seenSrc.push(val);
      seenDst.push(abCopy);
      return abCopy;
    }
    if (val.buffer instanceof ArrayBuffer && typeof val.byteLength === "number") {
      var Ctor = val.constructor;
      var sliceBuf = val.buffer.slice(val.byteOffset || 0, (val.byteOffset || 0) + val.byteLength);
      var viewCopy = new Ctor(sliceBuf);
      seenSrc.push(val);
      seenDst.push(viewCopy);
      return viewCopy;
    }
    if (val._isFile) {
      if (!globalThis.File) __slice_ensure_shard(15);
      var fCopy = new globalThis.File([new Uint8Array(val._u8)], val.name, {
        type: val.type,
        lastModified: val.lastModified
      });
      seenSrc.push(val);
      seenDst.push(fCopy);
      return fCopy;
    }
    if (val._isBlob) {
      if (!globalThis.Blob) __slice_ensure_shard(15);
      var bCopy = new globalThis.Blob([new Uint8Array(val._u8)], { type: val.type });
      seenSrc.push(val);
      seenDst.push(bCopy);
      return bCopy;
    }
    if (val instanceof Map) {
      var mCopy = new Map();
      seenSrc.push(val);
      seenDst.push(mCopy);
      val.forEach(function(v, k) {
        mCopy.set(__cloneDeep(k, seenSrc, seenDst), __cloneDeep(v, seenSrc, seenDst));
      });
      return mCopy;
    }
    if (val instanceof Set) {
      var sCopy = new Set();
      seenSrc.push(val);
      seenDst.push(sCopy);
      val.forEach(function(v) {
        sCopy.add(__cloneDeep(v, seenSrc, seenDst));
      });
      return sCopy;
    }
    if (val instanceof Error) {
      var eCopy = new Error(val.message);
      eCopy.name = val.name;
      if (val.stack) eCopy.stack = val.stack;
      seenSrc.push(val);
      seenDst.push(eCopy);
      return eCopy;
    }
    if (Array.isArray(val)) {
      var arrCopy = new Array(val.length);
      seenSrc.push(val);
      seenDst.push(arrCopy);
      for (var j = 0; j < val.length; j++) {
        arrCopy[j] = __cloneDeep(val[j], seenSrc, seenDst);
      }
      return arrCopy;
    }
    var objCopy = {};
    seenSrc.push(val);
    seenDst.push(objCopy);
    var keys = Object.keys(val);
    for (var k = 0; k < keys.length; k++) {
      var prop = keys[k];
      objCopy[prop] = __cloneDeep(val[prop], seenSrc, seenDst);
    }
    return objCopy;
  }

  globalThis.structuredClone = function(value) {
    if (arguments.length === 0) throw new TypeError("structuredClone requires at least 1 argument");
    return __cloneDeep(value, [], []);
  };
})();

