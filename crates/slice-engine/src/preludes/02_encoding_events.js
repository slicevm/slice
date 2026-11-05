// Slice (Nexus-JS) Lazy Prelude Shard 2: Encoding (`btoa`, `atob`, `TextEncoder`, `TextDecoder`) (`02_encoding_events.js`)
(function() {
  globalThis.btoa = function(s) {
    if (arguments.length === 0) throw new TypeError("btoa requires at least 1 argument");
    return __slice_btoa(String(s));
  };

  globalThis.atob = function(s) {
    if (arguments.length === 0) throw new TypeError("atob requires at least 1 argument");
    return __slice_atob(String(s));
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

  function TextDecoder(label, options) {
    var o = options || {};
    this.encoding = __normEncLabel(label);
    this.fatal = Boolean(o.fatal);
    this.ignoreBOM = Boolean(o.ignoreBOM);
  }
  TextDecoder.prototype.decode = function(input) {
    if (input === undefined || input === null) return "";
    return __slice_text_decode(input, this.encoding, this.fatal, this.ignoreBOM);
  };
  globalThis.TextDecoder = TextDecoder;
})();
