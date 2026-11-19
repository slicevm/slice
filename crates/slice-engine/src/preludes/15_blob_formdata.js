// Slice (Nexus-JS) Lazy Prelude Shard 15: WHATWG `Blob`, `File`, `FormData` (`15_blob_formdata.js`)
(function() {
  if (!globalThis.TextDecoder) __slice_ensure_shard(2);

  var __IterProto = Object.getPrototypeOf(Object.getPrototypeOf([][Symbol.iterator]()));

  function __sanitizeMimeType(raw) {
    if (!raw) return "";
    var s = String(raw);
    for (var i = 0; i < s.length; i++) {
      var c = s.charCodeAt(i);
      if (c < 0x20 || c > 0x7E) return "";
    }
    return s.toLowerCase();
  }

  function Blob(blobParts, options) {
    var o = options || {};
    this.type = o.type !== undefined ? __sanitizeMimeType(o.type) : "";
    var chunks = [];
    if (blobParts !== undefined && blobParts !== null) {
      if (!Array.isArray(blobParts) && typeof blobParts.length !== "number") {
        throw new TypeError("Blob parts must be an sequence/array");
      }
      for (var i = 0; i < blobParts.length; i++) {
        chunks.push(__slice_toU8(blobParts[i]));
      }
    }
    this._u8 = __slice_concatU8(chunks);
    this.size = this._u8.byteLength;
  }
  Blob.prototype._isBlob = true;
  Blob.prototype.text = function() {
    return Promise.resolve(__slice_text_decode(this._u8, "utf-8", false, false));
  };
  Blob.prototype.arrayBuffer = function() {
    var copy = new Uint8Array(this._u8);
    return Promise.resolve(copy.buffer);
  };
  Blob.prototype.bytes = function() {
    return Promise.resolve(new Uint8Array(this._u8));
  };
  Blob.prototype.slice = function(start, end, contentType) {
    var len = this.size;
    var s = start === undefined ? 0 : (start < 0 ? Math.max(len + Number(start), 0) : Math.min(Number(start), len));
    var e = end === undefined ? len : (end < 0 ? Math.max(len + Number(end), 0) : Math.min(Number(end), len));
    var sub = e > s ? this._u8.subarray(s, e) : new Uint8Array(0);
    return new Blob([sub], { type: contentType !== undefined ? __sanitizeMimeType(contentType) : "" });
  };
  Blob.prototype.stream = function() {
    if (!globalThis.ReadableStream) __slice_ensure_shard(3);
    var u8 = new Uint8Array(this._u8);
    return new globalThis.ReadableStream({
      start: function(c) {
        if (u8.byteLength > 0) c.enqueue(u8);
        c.close();
      }
    });
  };
  globalThis.Blob = Blob;

  function File(fileBits, fileName, options) {
    if (arguments.length < 2) throw new TypeError("File constructor requires at least 2 arguments");
    Blob.call(this, fileBits, options);
    this.name = String(fileName);
    this.lastModified = (options && options.lastModified !== undefined) ? Number(options.lastModified) : Date.now();
  }
  File.prototype = Object.create(Blob.prototype);
  File.prototype.constructor = File;
  File.prototype._isFile = true;
  globalThis.File = File;

  function FormData() {
    this._p = [];
  }
  FormData.prototype._isFormData = true;
  FormData.prototype._normVal = function(value, filename) {
    if (value instanceof File) {
      return filename !== undefined
        ? new File([value._u8], filename, { type: value.type, lastModified: value.lastModified })
        : value;
    }
    if (value instanceof Blob) {
      return new File([value._u8], filename !== undefined ? String(filename) : "blob", { type: value.type });
    }
    return String(value);
  };
  FormData.prototype.append = function(name, value, filename) {
    this._p.push([String(name), this._normVal(value, filename)]);
  };
  FormData.prototype.set = function(name, value, filename) {
    var k = String(name), v = this._normVal(value, filename), fi = -1;
    for (var i = 0; i < this._p.length; i++) {
      if (this._p[i][0] === k) {
        if (fi === -1) {
          this._p[i][1] = v;
          fi = i;
        } else {
          this._p.splice(i, 1);
          i--;
        }
      }
    }
    if (fi === -1) this._p.push([k, v]);
  };
  FormData.prototype.get = function(name) {
    var k = String(name);
    for (var i = 0; i < this._p.length; i++) {
      if (this._p[i][0] === k) return this._p[i][1];
    }
    return null;
  };
  FormData.prototype.getAll = function(name) {
    var k = String(name), r = [];
    for (var i = 0; i < this._p.length; i++) {
      if (this._p[i][0] === k) r.push(this._p[i][1]);
    }
    return r;
  };
  FormData.prototype.has = function(name) {
    return this.get(name) !== null;
  };
  FormData.prototype.delete = function(name) {
    var k = String(name), nx = [];
    for (var i = 0; i < this._p.length; i++) {
      if (this._p[i][0] !== k) nx.push(this._p[i]);
    }
    this._p = nx;
  };
  var __fdIterProto = Object.create(__IterProto);
  __fdIterProto.next = function() {
    var p = this._t._p;
    if (this._i >= p.length) return { value: undefined, done: true };
    var pair = p[this._i++];
    var k = this._k;
    return {
      value: k === 0 ? pair[0] : (k === 1 ? pair[1] : [pair[0], pair[1]]),
      done: false
    };
  };
  if (typeof Symbol !== "undefined" && Symbol.toStringTag) {
    Object.defineProperty(__fdIterProto, Symbol.toStringTag, {
      value: "FormData Iterator",
      configurable: true
    });
  }
  function __mkFdIter(fd, kind) {
    var it = Object.create(__fdIterProto);
    it._t = fd;
    it._i = 0;
    it._k = kind;
    return it;
  }

  FormData.prototype.entries = function() {
    return __mkFdIter(this, 2);
  };
  FormData.prototype.keys = function() {
    return __mkFdIter(this, 0);
  };
  FormData.prototype.values = function() {
    return __mkFdIter(this, 1);
  };
  FormData.prototype.forEach = function(cb, thisArg) {
    for (var i = 0; i < this._p.length; i++) cb.call(thisArg, this._p[i][1], this._p[i][0], this);
  };
  if (typeof Symbol !== "undefined" && Symbol.iterator) {
    FormData.prototype[Symbol.iterator] = FormData.prototype.entries;
  }
  globalThis.FormData = FormData;

  function __escapeMpAttr(s) {
    return String(s)
      .replace(/\r/g, "%0D")
      .replace(/\n/g, "%0A")
      .replace(/"/g, "%22");
  }

  function __formDataContainsBoundary(es, boundary) {
    for (var i = 0; i < es.length; i++) {
      var escName = __escapeMpAttr(es[i][0]);
      if (escName.indexOf(boundary) !== -1) return true;
      var v = es[i][1];
      if (v && v._isBlob) {
        var fn = __escapeMpAttr(v.name !== undefined ? v.name : "blob");
        if (fn.indexOf(boundary) !== -1) return true;
        if (__slice_u8_contains(v._u8, boundary)) return true;
      } else if (String(v).indexOf(boundary) !== -1) {
        return true;
      }
    }
    return false;
  }

  globalThis.__slice_encode_formdata = function(rawBody, headers) {
    var es = rawBody._p;
    var boundary = __slice_multipart_boundary();
    for (var attempt = 0; attempt < 16 && __formDataContainsBoundary(es, boundary); attempt++) {
      boundary = __slice_multipart_boundary();
    }
    if (headers && !headers.has("content-type")) {
      headers.set("content-type", "multipart/form-data; boundary=" + boundary);
    }
    var chunks = [];
    for (var i = 0; i < es.length; i++) {
      var escName = __escapeMpAttr(es[i][0]);
      var v = es[i][1];
      if (v && v._isBlob) {
        var fn = __escapeMpAttr(v.name !== undefined ? v.name : "blob");
        var bType = __sanitizeMimeType(v.type) || "application/octet-stream";
        chunks.push(
          __slice_text_encode(
            "--" + boundary + "\r\nContent-Disposition: form-data; name=\"" + escName + "\"; filename=\"" + fn + "\"\r\nContent-Type: " + bType + "\r\n\r\n"
          )
        );
        chunks.push(new Uint8Array(v._u8));
        chunks.push(__slice_text_encode("\r\n"));
      } else {
        chunks.push(
          __slice_text_encode(
            "--" + boundary + "\r\nContent-Disposition: form-data; name=\"" + escName + "\"\r\n\r\n" + String(v) + "\r\n"
          )
        );
      }
    }
    chunks.push(__slice_text_encode("--" + boundary + "--\r\n"));
    return {
      str: null,
      u8: __slice_concatU8(chunks)
    };
  };

  globalThis.__slice_parse_formdata = function(obj) {
    var ct = (obj && obj.headers && typeof obj.headers.get === "function")
      ? (obj.headers.get("content-type") || "")
      : "";
    if (/^\s*multipart\/form-data(?:\s*;|$)/i.test(ct)) {
      var bMatch = /;\s*boundary=(?:"([^"]*)"|([^\s;]+))/i.exec(ct);
      var boundary = bMatch ? (bMatch[1] !== undefined ? bMatch[1] : bMatch[2]) : "";
      if (!boundary) {
        throw new TypeError("Failed to parse multipart/form-data: missing boundary parameter");
      }
      var rawBytes = (obj && typeof obj._borrowBytes === "function")
        ? obj._borrowBytes()
        : __slice_text_encode((obj && obj._body) || "");
      var rawParts = __slice_multipart_parse(rawBytes, boundary);
      var fd = new FormData();
      var fdPairs = fd._p;
      var nowMs = Date.now();
      for (var p = 0; p < rawParts.length; p++) {
        var item = rawParts[p];
        if (item[1]) {
          var fObj = Object.create(File.prototype);
          fObj.type = __sanitizeMimeType(item[3]);
          fObj._u8 = item[4];
          fObj.size = item[4].byteLength;
          fObj.name = String(item[2]);
          fObj.lastModified = nowMs;
          fdPairs.push([String(item[0]), fObj]);
        } else {
          fdPairs.push([String(item[0]), String(item[2])]);
        }
      }
      return fd;
    }
    if (/^\s*application\/x-www-form-urlencoded(?:\s*;|$)/i.test(ct)) {
      if (!globalThis.URLSearchParams) __slice_ensure_shard(1);
      var rawText = (obj && typeof obj._getText === "function")
        ? obj._getText()
        : ((obj && obj._body) || "");
      var qsInput = rawText.charCodeAt(0) === 63 ? ("?" + rawText) : rawText;
      var usp = new globalThis.URLSearchParams(qsInput);
      var fdUrl = new FormData();
      var es = usp._ensureP();
      for (var i = 0; i < es.length; i++) fdUrl.append(es[i][0], es[i][1]);
      return fdUrl;
    }
    throw new TypeError(
      "Content-Type was not one of \"multipart/form-data\" or \"application/x-www-form-urlencoded\""
    );
  };
})();

