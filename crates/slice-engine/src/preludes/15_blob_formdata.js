// Slice (Nexus-JS) Lazy Prelude Shard 15: WHATWG `Blob`, `File`, `FormData` (`15_blob_formdata.js`)
(function() {
  if (!globalThis.TextDecoder) __slice_ensure_shard(2);

  function Blob(blobParts, options) {
    var o = options || {};
    this.type = o.type ? String(o.type).toLowerCase() : "";
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
    return new Blob([sub], { type: contentType !== undefined ? String(contentType) : "" });
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
  FormData.prototype.entries = function() {
    return this._p.slice();
  };
  FormData.prototype.keys = function() {
    var r = [];
    for (var i = 0; i < this._p.length; i++) r.push(this._p[i][0]);
    return r;
  };
  FormData.prototype.values = function() {
    var r = [];
    for (var i = 0; i < this._p.length; i++) r.push(this._p[i][1]);
    return r;
  };
  FormData.prototype.forEach = function(cb, thisArg) {
    for (var i = 0; i < this._p.length; i++) cb.call(thisArg, this._p[i][1], this._p[i][0], this);
  };
  if (typeof Symbol !== "undefined" && Symbol.iterator) {
    FormData.prototype[Symbol.iterator] = function() {
      return this._p[Symbol.iterator]();
    };
  }
  globalThis.FormData = FormData;

  function __escapeMpAttr(s) {
    return String(s)
      .replace(/\r/g, "%0D")
      .replace(/\n/g, "%0A")
      .replace(/"/g, "%22");
  }

  function __unescapeMpAttr(s) {
    return String(s)
      .replace(/%22/gi, "\"")
      .replace(/%0A/gi, "\n")
      .replace(/%0D/gi, "\r");
  }

  globalThis.__slice_encode_formdata = function(rawBody, headers) {
    var boundary = "----SliceFormBoundary";
    if (headers && !headers.has("content-type")) {
      headers.set("content-type", "multipart/form-data; boundary=" + boundary);
    }
    var chunks = [], es = rawBody.entries();
    for (var i = 0; i < es.length; i++) {
      var escName = __escapeMpAttr(es[i][0]);
      var v = es[i][1];
      if (v && v._isBlob) {
        var fn = __escapeMpAttr(v.name !== undefined ? v.name : "blob");
        var bType = v.type || "application/octet-stream";
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
    var mpU8 = __slice_concatU8(chunks);
    return {
      str: __slice_text_decode(mpU8, "utf-8", false, false),
      u8: mpU8,
      fd: rawBody
    };
  };

  globalThis.__slice_parse_formdata = function(obj) {
    var fd = new FormData();
    var ct = (obj._ct || (obj.headers && obj.headers.get("content-type")) || "");
    var bMatch = /multipart\/form-data\s*;\s*boundary=(?:"([^"]+)"|([^\s;]+))/i.exec(ct);
    var rawText = obj._body || "";
    if (bMatch) {
      var boundary = "--" + (bMatch[1] || bMatch[2]);
      var parts = rawText.split(boundary);
      for (var p = 1; p < parts.length; p++) {
        var seg = parts[p];
        if (seg.indexOf("--") === 0) break;
        if (seg.indexOf("\r\n") === 0) seg = seg.slice(2);
        else if (seg.indexOf("\n") === 0) seg = seg.slice(1);
        var sep = seg.indexOf("\r\n\r\n");
        var sepLen = 4;
        if (sep === -1) {
          sep = seg.indexOf("\n\n");
          sepLen = 2;
        }
        if (sep === -1) continue;
        var hdrBlock = seg.slice(0, sep);
        var valStr = seg.slice(sep + sepLen);
        if (valStr.slice(-2) === "\r\n") valStr = valStr.slice(0, -2);
        else if (valStr.slice(-1) === "\n") valStr = valStr.slice(0, -1);
        var nMatch = /name="([^"]*)"/i.exec(hdrBlock);
        if (!nMatch) continue;
        var fieldName = __unescapeMpAttr(nMatch[1]);
        var fnMatch = /filename="([^"]*)"/i.exec(hdrBlock);
        if (fnMatch) {
          var ctMatch = /content-type:\s*([^\r\n]+)/i.exec(hdrBlock);
          var fType = ctMatch ? ctMatch[1].trim() : "application/octet-stream";
          fd.append(fieldName, new File([valStr], __unescapeMpAttr(fnMatch[1]), { type: fType }));
        } else {
          fd.append(fieldName, valStr);
        }
      }
      return fd;
    }
    if (!globalThis.URLSearchParams) __slice_ensure_shard(1);
    var usp = new globalThis.URLSearchParams(rawText);
    var es = usp.entries();
    for (var i = 0; i < es.length; i++) fd.append(es[i][0], es[i][1]);
    return fd;
  };
})();

