// Slice (Nexus-JS) Lazy Prelude Shard 17: `Buffer.prototype` Binary Integer Read/Write & Slice Mutators (`17_buffer_binary_rw.js`)
(function() {
  if (!globalThis.Buffer) __slice_ensure_shard(5);
  var Buffer = globalThis.Buffer;
  var __U8 = globalThis.Uint8Array;
  var __u8Proto = __U8.prototype;

  Buffer.prototype._hasBinRW = true;

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
  Buffer.prototype.readBigInt64BE = function(offset) {
    var hi = BigInt(this.readInt32BE(offset));
    var lo = BigInt(this.readUInt32BE((offset | 0) + 4));
    return (hi << 32n) | lo;
  };
  Buffer.prototype.readBigInt64LE = function(offset) {
    var lo = BigInt(this.readUInt32LE(offset));
    var hi = BigInt(this.readInt32LE((offset | 0) + 4));
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
  Buffer.prototype.writeBigInt64BE = Buffer.prototype.writeBigUInt64BE;
  Buffer.prototype.writeBigInt64LE = Buffer.prototype.writeBigUInt64LE;
})();

