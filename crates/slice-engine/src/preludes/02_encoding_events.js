// Slice (Nexus-JS) Lazy Prelude Shard 2: Encoding (`btoa`, `atob`, `TextEncoder`, `TextDecoder`) (`02_encoding_events.js`)
(function() {
  function __throwInvalidChar(e) {
    var msg = e && e.message ? String(e.message) : "InvalidCharacterError";
    if (!globalThis.DOMException) __slice_ensure_shard(13);
    throw new globalThis.DOMException(msg, "InvalidCharacterError");
  }

  globalThis.btoa = function(s) {
    if (arguments.length === 0) throw new TypeError("btoa requires at least 1 argument");
    try {
      return __slice_btoa(typeof s === "string" ? s : String(s));
    } catch (e) {
      __throwInvalidChar(e);
    }
  };

  globalThis.atob = function(s) {
    if (arguments.length === 0) throw new TypeError("atob requires at least 1 argument");
    try {
      return __slice_atob(typeof s === "string" ? s : String(s));
    } catch (e) {
      __throwInvalidChar(e);
    }
  };

  function TextEncoder() {
    this.encoding = "utf-8";
  }
  TextEncoder.prototype.encode = function(input) {
    return __slice_text_encode(input === undefined ? "" : (typeof input === "string" ? input : String(input)));
  };
  TextEncoder.prototype.encodeInto = function(src, dest) {
    return __slice_text_encode_into(typeof src === "string" ? src : String(src), dest);
  };
  globalThis.TextEncoder = TextEncoder;

  function __normEncLabel(label) {
    if (label === undefined) return "utf-8";
    var s = String(label).trim().toLowerCase();
    if (s === "utf-8" || s === "utf8" || s === "unicode-1-1-utf-8") return "utf-8";
    if (s === "ascii" || s === "us-ascii") return "ascii";
    if (s === "latin1" || s === "iso-8859-1" || s === "windows-1252") return "iso-8859-1";
    throw new RangeError("TextDecoder: Unsupported encoding label '" + label + "'");
  }

  function __utf8IncompleteTailIndex(u8) {
    var len = u8.byteLength;
    if (len === 0) return 0;
    var cont = 0;
    while (cont < 3 && cont < len && (u8[len - 1 - cont] & 0xC0) === 0x80) {
      cont++;
    }
    if (cont === len || cont === 3) return len;
    var leadIdx = len - 1 - cont;
    var lead = u8[leadIdx];
    var needed = 0;
    if (lead >= 0xC2 && lead <= 0xDF) needed = 2;
    else if (lead >= 0xE0 && lead <= 0xEF) needed = 3;
    else if (lead >= 0xF0 && lead <= 0xF4) needed = 4;
    if (needed > cont + 1) return leadIdx;
    return len;
  }

  function TextDecoder(label, options) {
    var o = options || {};
    this.encoding = __normEncLabel(label);
    this.fatal = Boolean(o.fatal);
    this.ignoreBOM = Boolean(o.ignoreBOM);
    this._pending = null;
    this._bomSeen = false;
  }
  TextDecoder.prototype.decode = function(input, options) {
    var stream = Boolean(options && options.stream);
    if (!stream && this._pending === null) {
      this._bomSeen = false;
      if (input === undefined || input === null) return "";
      return __slice_text_decode(input, this.encoding, this.fatal, this.ignoreBOM);
    }
    var chunk = (input === undefined || input === null) ? new Uint8Array(0) : __slice_toU8(input);
    var combined = chunk;
    if (this._pending !== null) {
      if (chunk.byteLength === 0) {
        combined = this._pending;
      } else {
        combined = new Uint8Array(this._pending.byteLength + chunk.byteLength);
        combined.set(this._pending, 0);
        combined.set(chunk, this._pending.byteLength);
      }
      this._pending = null;
    }
    if (stream && this.encoding === "utf-8" && combined.byteLength > 0) {
      var splitIdx = __utf8IncompleteTailIndex(combined);
      if (splitIdx < combined.byteLength) {
        this._pending = combined.slice(splitIdx);
        combined = combined.subarray(0, splitIdx);
      }
    }
    var igBom = this.ignoreBOM || this._bomSeen;
    if (combined.byteLength === 0) {
      if (!stream) this._bomSeen = false;
      return "";
    }
    var out = __slice_text_decode(combined, this.encoding, this.fatal, igBom);
    if (stream) {
      this._bomSeen = true;
    } else {
      this._bomSeen = false;
    }
    return out;
  };
  globalThis.TextDecoder = TextDecoder;
})();
