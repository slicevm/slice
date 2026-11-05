// Slice (Nexus-JS) Lazy Prelude Shard 5: Core Node.js `Buffer` (Uint8Array-backed) (`05_full_buffer.js`)
(function() {
  var __U8 = globalThis.Uint8Array;
  var __u8Proto = __U8.prototype;

  function __wrapBuf(u8) {
    Object.setPrototypeOf(u8, Buffer.prototype);
    return u8;
  }
  globalThis.__slice_wrapBuf = __wrapBuf;

  function Buffer(arg, encOrOffset, length) {
    if (typeof arg === "number") return Buffer.alloc(arg);
    return Buffer.from(arg, encOrOffset, length);
  }
  Object.setPrototypeOf(Buffer.prototype, __u8Proto);
  Object.setPrototypeOf(Buffer, __U8);
  Buffer.prototype._isBuffer = true;
  Buffer.poolSize = 8192;

  Buffer.isBuffer = function(obj) {
    return Boolean(obj && obj._isBuffer === true);
  };

  Buffer.isEncoding = function(enc) {
    if (typeof enc !== "string") return false;
    var e = enc.toLowerCase();
    return e === "utf8" || e === "utf-8" || e === "hex" || e === "base64" || e === "base64url" ||
           e === "latin1" || e === "binary" || e === "ascii" || e === "ucs2" || e === "ucs-2" ||
           e === "utf16le" || e === "utf-16le";
  };

  Buffer.from = function(value, encOrOffset, length) {
    if (typeof value === "string") {
      var e = encOrOffset ? String(encOrOffset).toLowerCase() : "utf8";
      if (e === "base64" || e === "base64url") {
        return __wrapBuf(__slice_base64_decode_bytes(value));
      }
      if (e === "hex") {
        return __wrapBuf(__slice_hex_decode(value));
      }
      if (e === "latin1" || e === "binary" || e === "ascii") {
        var n = value.length, u = new __U8(n);
        var mask = e === "ascii" ? 0x7f : 0xff;
        for (var i = 0; i < n; i++) u[i] = value.charCodeAt(i) & mask;
        return __wrapBuf(u);
      }
      return __wrapBuf(__slice_text_encode(value));
    }
    if (value instanceof ArrayBuffer) {
      var off = encOrOffset !== undefined ? Math.max(0, Number(encOrOffset) | 0) : 0;
      var maxLen = Math.max(0, value.byteLength - off);
      var len2 = length !== undefined ? Math.min(maxLen, Math.max(0, Number(length) | 0)) : maxLen;
      return __wrapBuf(new __U8(value, off, len2));
    }
    if (value instanceof __U8) {
      return __wrapBuf(new __U8(value));
    }
    if (value && ArrayBuffer.isView(value)) {
      return __wrapBuf(new __U8(value.buffer.slice(value.byteOffset, value.byteOffset + value.byteLength)));
    }
    if (Array.isArray(value) || (value && typeof value.length === "number")) {
      return __wrapBuf(new __U8(value));
    }
    if (value && value.type === "Buffer" && Array.isArray(value.data)) {
      return __wrapBuf(new __U8(value.data));
    }
    throw new TypeError("The first argument must be of type string, Buffer, ArrayBuffer, Array, or Array-like Object");
  };

  Buffer.alloc = function(size, fill, encoding) {
    var n = Math.max(0, Number(size) | 0);
    var b = __wrapBuf(new __U8(n));
    if (fill !== undefined && fill !== 0 && n > 0) {
      b.fill(fill, 0, n, encoding);
    }
    return b;
  };

  Buffer.allocUnsafe = function(size) {
    return Buffer.alloc(size);
  };

  Buffer.allocUnsafeSlow = function(size) {
    return Buffer.alloc(size);
  };

  Buffer.byteLength = __slice_utf8_byte_len;

  Buffer.concat = function(list, totalLength) {
    if (!Array.isArray(list)) throw new TypeError("The \"list\" argument must be an instance of Array");
    if (list.length === 0) return Buffer.alloc(0);
    var total = 0;
    if (totalLength !== undefined) {
      total = Math.max(0, Number(totalLength) | 0);
    } else {
      for (var i = 0; i < list.length; i++) total += list[i].length;
    }
    var out = Buffer.alloc(total);
    var pos = 0;
    for (var j = 0; j < list.length; j++) {
      var chunk = list[j];
      if (!chunk || chunk.length === 0) continue;
      var rem = total - pos;
      if (rem <= 0) break;
      if (chunk.length <= rem) {
        out.set(chunk, pos);
        pos += chunk.length;
      } else {
        out.set(__u8Proto.subarray.call(chunk, 0, rem), pos);
        pos += rem;
      }
    }
    return out;
  };

  Buffer.compare = function(a, b) {
    if (!(a instanceof __U8) || !(b instanceof __U8)) {
      throw new TypeError("The \"buf1\", \"buf2\" arguments must be Uint8Array or Buffer");
    }
    var len = Math.min(a.length, b.length);
    for (var i = 0; i < len; i++) {
      if (a[i] !== b[i]) return a[i] < b[i] ? -1 : 1;
    }
    return a.length === b.length ? 0 : (a.length < b.length ? -1 : 1);
  };

  Buffer.prototype.toString = function(encoding, start, end) {
    var len = this.length;
    if (len === 0) return "";
    var s = start === undefined ? 0 : Math.max(0, Number(start) | 0);
    var e = end === undefined ? len : Math.min(len, Number(end) | 0);
    if (s >= e) return "";
    var view = (s === 0 && e === len) ? this : __u8Proto.subarray.call(this, s, e);
    var enc = encoding ? String(encoding).toLowerCase() : "utf8";
    if (enc === "hex") return __slice_bytes_to_hex(view);
    if (enc === "base64") return __slice_base64_encode_bytes(view, false);
    if (enc === "base64url") return __slice_base64_encode_bytes(view, true);
    if (enc === "latin1" || enc === "binary") return __slice_text_decode(view, "latin1", false, true);
    if (enc === "ascii") {
      var out = "";
      for (var i = 0; i < view.length; i++) out += String.fromCharCode(view[i] & 0x7f);
      return out;
    }
    return __slice_text_decode(view, "utf-8", false, true);
  };

  Buffer.prototype.subarray = function(start, end) {
    return __wrapBuf(__u8Proto.subarray.call(this, start, end));
  };

  Buffer.prototype.slice = function(start, end) {
    return __wrapBuf(__u8Proto.subarray.call(this, start, end));
  };

  Buffer.prototype.equals = function(other) {
    if (!(other instanceof __U8)) {
      throw new TypeError("The \"otherBuffer\" argument must be an instance of Buffer or Uint8Array");
    }
    return Buffer.compare(this, other) === 0;
  };

  var __binM = [
    "compare","copy","fill","write","toJSON",
    "readUInt8","readInt8","readUInt16BE","readUInt16LE","readInt16BE","readInt16LE",
    "readUInt32BE","readUInt32LE","readInt32BE","readInt32LE",
    "readBigUInt64BE","readBigUInt64LE","readBigInt64BE","readBigInt64LE",
    "writeUInt8","writeInt8","writeUInt16BE","writeUInt16LE","writeInt16BE","writeInt16LE",
    "writeUInt32BE","writeUInt32LE","writeInt32BE","writeInt32LE",
    "writeBigUInt64BE","writeBigUInt64LE","writeBigInt64BE","writeBigInt64LE"
  ];
  for (var bi = 0; bi < __binM.length; bi++) {
    (function(m) {
      Buffer.prototype[m] = function() {
        __slice_ensure_shard(17);
        return Buffer.prototype[m].apply(this, arguments);
      };
    })(__binM[bi]);
  }

  globalThis.Buffer = Buffer;
})();
