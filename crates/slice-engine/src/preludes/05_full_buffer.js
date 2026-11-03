// Slice (Nexus-JS) Lazy Prelude Shard 5: Full Node.js `Buffer` (Uint8Array-backed, 60+ methods) (`05_full_buffer.js`)
(function() {
  var __U8 = globalThis.Uint8Array;
  var __u8Proto = __U8.prototype;

  function __wrapBuf(u8) {
    Object.setPrototypeOf(u8, Buffer.prototype);
    return u8;
  }

  function __utf8ByteLen(str) {
    var s = String(str), len = s.length, bytes = 0;
    for (var i = 0; i < len; i++) {
      var c = s.charCodeAt(i);
      if (c < 0x80) bytes += 1;
      else if (c < 0x800) bytes += 2;
      else if (c >= 0xD800 && c <= 0xDBFF && i + 1 < len) {
        var c2 = s.charCodeAt(i + 1);
        if (c2 >= 0xDC00 && c2 <= 0xDFFF) {
          bytes += 4;
          i++;
        } else {
          bytes += 3;
        }
      } else {
        bytes += 3;
      }
    }
    return bytes;
  }

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

  Buffer.byteLength = function(input, encoding) {
    if (typeof input !== "string") {
      if (input && typeof input.byteLength === "number") return input.byteLength;
      if (input && typeof input.length === "number") return input.length;
      input = String(input);
    }
    var e = encoding ? String(encoding).toLowerCase() : "utf8";
    if (e === "utf8" || e === "utf-8") return __utf8ByteLen(input);
    if (e === "hex") return input.length >>> 1;
    if (e === "base64" || e === "base64url") return __slice_base64_decode_bytes(input).byteLength;
    if (e === "latin1" || e === "binary" || e === "ascii") return input.length;
    return __utf8ByteLen(input);
  };

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

  Buffer.prototype.compare = function(target, targetStart, targetEnd, sourceStart, sourceEnd) {
    if (!(target instanceof __U8)) {
      throw new TypeError("The \"target\" argument must be an instance of Buffer or Uint8Array");
    }
    var ts = targetStart === undefined ? 0 : Math.max(0, Number(targetStart) | 0);
    var te = targetEnd === undefined ? target.length : Math.min(target.length, Number(targetEnd) | 0);
    var ss = sourceStart === undefined ? 0 : Math.max(0, Number(sourceStart) | 0);
    var se = sourceEnd === undefined ? this.length : Math.min(this.length, Number(sourceEnd) | 0);
    var a = (ss === 0 && se === this.length) ? this : __u8Proto.subarray.call(this, ss, se);
    var b = (ts === 0 && te === target.length) ? target : __u8Proto.subarray.call(target, ts, te);
    return Buffer.compare(a, b);
  };

  Buffer.prototype.copy = function(target, targetStart, sourceStart, sourceEnd) {
    if (!(target instanceof __U8)) {
      throw new TypeError("The \"target\" argument must be a Buffer or Uint8Array");
    }
    var ts = targetStart === undefined ? 0 : Math.max(0, Number(targetStart) | 0);
    var ss = sourceStart === undefined ? 0 : Math.max(0, Number(sourceStart) | 0);
    var se = sourceEnd === undefined ? this.length : Math.min(this.length, Number(sourceEnd) | 0);
    if (ss >= se || ts >= target.length) return 0;
    var count = Math.min(se - ss, target.length - ts);
    target.set(__u8Proto.subarray.call(this, ss, ss + count), ts);
    return count;
  };

  Buffer.prototype.fill = function(val, offset, end, encoding) {
    var s = offset === undefined ? 0 : Math.max(0, Number(offset) | 0);
    var e = end === undefined ? this.length : Math.min(this.length, Number(end) | 0);
    if (s >= e) return this;
    if (typeof val === "number") {
      __u8Proto.fill.call(this, val & 0xff, s, e);
      return this;
    }
    var pat = typeof val === "string"
      ? Buffer.from(val, encoding)
      : (val instanceof __U8 ? val : Buffer.from([Number(val) & 0xff]));
    if (pat.length === 0) {
      __u8Proto.fill.call(this, 0, s, e);
      return this;
    }
    for (var i = s; i < e; i++) {
      this[i] = pat[(i - s) % pat.length];
    }
    return this;
  };

  Buffer.prototype.write = function(str, offset, length, encoding) {
    var off = 0, len = this.length, enc = "utf8";
    if (typeof offset === "string") {
      enc = offset;
    } else if (typeof offset === "number") {
      off = offset | 0;
      if (typeof length === "string") enc = length;
      else if (typeof length === "number") {
        len = length | 0;
        if (typeof encoding === "string") enc = encoding;
      }
    }
    var maxAvail = Math.max(0, this.length - off);
    var maxWrite = Math.min(maxAvail, Math.max(0, len));
    if (maxWrite === 0) return 0;
    var src = Buffer.from(String(str), enc);
    var count = Math.min(src.length, maxWrite);
    this.set(__u8Proto.subarray.call(src, 0, count), off);
    return count;
  };

  Buffer.prototype.toJSON = function() {
    var d = new Array(this.length);
    for (var i = 0; i < this.length; i++) d[i] = this[i];
    return { type: "Buffer", data: d };
  };

  Buffer.prototype.readUInt8 = function(offset) {
    var o = (offset | 0);
    if (o < 0 || o >= this.length) throw new RangeError("Index out of range");
    return this[o];
  };
  Buffer.prototype.readInt8 = function(offset) {
    var v = this.readUInt8(offset);
    return (v & 0x80) ? v - 0x100 : v;
  };
  Buffer.prototype.readUInt16BE = function(offset) {
    var o = (offset | 0);
    if (o < 0 || o + 2 > this.length) throw new RangeError("Index out of range");
    return (this[o] << 8) | this[o + 1];
  };
  Buffer.prototype.readUInt16LE = function(offset) {
    var o = (offset | 0);
    if (o < 0 || o + 2 > this.length) throw new RangeError("Index out of range");
    return this[o] | (this[o + 1] << 8);
  };
  Buffer.prototype.readInt16BE = function(offset) {
    var v = this.readUInt16BE(offset);
    return (v & 0x8000) ? v - 0x10000 : v;
  };
  Buffer.prototype.readInt16LE = function(offset) {
    var v = this.readUInt16LE(offset);
    return (v & 0x8000) ? v - 0x10000 : v;
  };
  Buffer.prototype.readUInt32BE = function(offset) {
    var o = (offset | 0);
    if (o < 0 || o + 4 > this.length) throw new RangeError("Index out of range");
    return (this[o] * 0x1000000) + ((this[o + 1] << 16) | (this[o + 2] << 8) | this[o + 3]);
  };
  Buffer.prototype.readUInt32LE = function(offset) {
    var o = (offset | 0);
    if (o < 0 || o + 4 > this.length) throw new RangeError("Index out of range");
    return ((this[o] | (this[o + 1] << 8) | (this[o + 2] << 16)) + (this[o + 3] * 0x1000000));
  };
  Buffer.prototype.readInt32BE = function(offset) {
    var o = (offset | 0);
    if (o < 0 || o + 4 > this.length) throw new RangeError("Index out of range");
    return (this[o] << 24) | (this[o + 1] << 16) | (this[o + 2] << 8) | this[o + 3];
  };
  Buffer.prototype.readInt32LE = function(offset) {
    var o = (offset | 0);
    if (o < 0 || o + 4 > this.length) throw new RangeError("Index out of range");
    return this[o] | (this[o + 1] << 8) | (this[o + 2] << 16) | (this[o + 3] << 24);
  };
  Buffer.prototype.readBigUInt64BE = function(offset) {
    var hi = BigInt(this.readUInt32BE(offset));
    var lo = BigInt(this.readUInt32BE((offset | 0) + 4));
    return (hi << 32n) | lo;
  };
  Buffer.prototype.readBigUInt64LE = function(offset) {
    var lo = BigInt(this.readUInt32LE(offset));
    var hi = BigInt(this.readUInt32LE((offset | 0) + 4));
    return (hi << 32n) | lo;
  };
  Buffer.prototype.writeUInt8 = function(value, offset) {
    var o = (offset | 0);
    if (o < 0 || o >= this.length) throw new RangeError("Index out of range");
    this[o] = Number(value) & 0xff;
    return o + 1;
  };
  Buffer.prototype.writeInt8 = Buffer.prototype.writeUInt8;
  Buffer.prototype.writeUInt16BE = function(value, offset) {
    var o = (offset | 0), v = Number(value) & 0xffff;
    if (o < 0 || o + 2 > this.length) throw new RangeError("Index out of range");
    this[o] = (v >>> 8) & 0xff;
    this[o + 1] = v & 0xff;
    return o + 2;
  };
  Buffer.prototype.writeUInt16LE = function(value, offset) {
    var o = (offset | 0), v = Number(value) & 0xffff;
    if (o < 0 || o + 2 > this.length) throw new RangeError("Index out of range");
    this[o] = v & 0xff;
    this[o + 1] = (v >>> 8) & 0xff;
    return o + 2;
  };
  Buffer.prototype.writeInt16BE = Buffer.prototype.writeUInt16BE;
  Buffer.prototype.writeInt16LE = Buffer.prototype.writeUInt16LE;
  Buffer.prototype.writeUInt32BE = function(value, offset) {
    var o = (offset | 0), v = Number(value) >>> 0;
    if (o < 0 || o + 4 > this.length) throw new RangeError("Index out of range");
    this[o] = (v >>> 24) & 0xff;
    this[o + 1] = (v >>> 16) & 0xff;
    this[o + 2] = (v >>> 8) & 0xff;
    this[o + 3] = v & 0xff;
    return o + 4;
  };
  Buffer.prototype.writeUInt32LE = function(value, offset) {
    var o = (offset | 0), v = Number(value) >>> 0;
    if (o < 0 || o + 4 > this.length) throw new RangeError("Index out of range");
    this[o] = v & 0xff;
    this[o + 1] = (v >>> 8) & 0xff;
    this[o + 2] = (v >>> 16) & 0xff;
    this[o + 3] = (v >>> 24) & 0xff;
    return o + 4;
  };
  Buffer.prototype.writeInt32BE = Buffer.prototype.writeUInt32BE;
  Buffer.prototype.writeInt32LE = Buffer.prototype.writeUInt32LE;
  Buffer.prototype.writeBigUInt64BE = function(value, offset) {
    var o = (offset | 0), v = BigInt(value);
    this.writeUInt32BE(Number((v >> 32n) & 0xffffffffn), o);
    this.writeUInt32BE(Number(v & 0xffffffffn), o + 4);
    return o + 8;
  };
  Buffer.prototype.writeBigUInt64LE = function(value, offset) {
    var o = (offset | 0), v = BigInt(value);
    this.writeUInt32LE(Number(v & 0xffffffffn), o);
    this.writeUInt32LE(Number((v >> 32n) & 0xffffffffn), o + 4);
    return o + 8;
  };

  globalThis.Buffer = Buffer;
})();

