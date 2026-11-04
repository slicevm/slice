// Slice (Nexus-JS) Lazy Prelude Shard 3: WHATWG Streams, Blob, File, FormData (`03_streams_formdata.js`)
(function() {
  var MAX_STREAM_CHUNKS = 4096;

  function ReadableStream(underlyingSource) {
    var self = this;
    this._chunks = [];
    this._closed = false;
    this._errored = null;
    this.locked = false;
    this._src = underlyingSource || null;
    var ctrl = {
      desiredSize: 1,
      enqueue: function(chunk) {
        if (self._closed) throw new TypeError("ReadableStream is already closed");
        if (self._chunks.length >= MAX_STREAM_CHUNKS) {
          throw new RangeError("ReadableStream buffer limit exceeded (" + MAX_STREAM_CHUNKS + " chunks)");
        }
        self._chunks.push(chunk);
      },
      close: function() {
        self._closed = true;
      },
      error: function(e) {
        self._errored = e || new Error("ReadableStream error");
        self._closed = true;
      }
    };
    this._ctrl = ctrl;
    if (this._src && typeof this._src.start === "function") {
      this._src.start(ctrl);
    }
  }
  ReadableStream.from = function(iterable) {
    return new ReadableStream({
      start: function(c) {
        if (Array.isArray(iterable)) {
          for (var i = 0; i < iterable.length; i++) c.enqueue(iterable[i]);
        } else if (iterable && typeof iterable[Symbol.iterator] === "function") {
          var it = iterable[Symbol.iterator](), step = it.next();
          while (!step.done) {
            c.enqueue(step.value);
            step = it.next();
          }
        }
        c.close();
      }
    });
  };
  ReadableStream.prototype._drainSync = function() {
    var guard = 0;
    while (!this._closed && this._src && typeof this._src.pull === "function" && guard < MAX_STREAM_CHUNKS) {
      guard++;
      var before = this._chunks.length;
      this._src.pull(this._ctrl);
      if (this._chunks.length === before && !this._closed) break;
    }
  };
  ReadableStream.prototype._toText = function() {
    this._drainSync();
    var out = "";
    for (var i = 0; i < this._chunks.length; i++) {
      var c = this._chunks[i];
      if (typeof c === "string") out += c;
      else if (c instanceof Uint8Array || c instanceof ArrayBuffer || (c && c.buffer instanceof ArrayBuffer)) {
        out += __slice_text_decode(c, "utf-8", false, false);
      } else if (c !== undefined && c !== null) {
        out += String(c);
      }
    }
    return out;
  };
  ReadableStream.prototype._toBytes = function() {
    this._drainSync();
    var u8s = [];
    for (var i = 0; i < this._chunks.length; i++) u8s.push(__slice_toU8(this._chunks[i]));
    return __slice_concatU8(u8s);
  };
  ReadableStream.prototype.cancel = function(reason) {
    this._closed = true;
    this._chunks = [];
    if (this._src && typeof this._src.cancel === "function") {
      try { return Promise.resolve(this._src.cancel(reason)); } catch (e) { return Promise.reject(e); }
    }
    return Promise.resolve(undefined);
  };
  ReadableStream.prototype.getReader = function() {
    if (this.locked) throw new TypeError("ReadableStream is already locked to a reader");
    this.locked = true;
    var self = this, idx = 0;
    return {
      closed: Promise.resolve(undefined),
      read: function() {
        if (self._errored) return Promise.reject(self._errored);
        if (idx < self._chunks.length) {
          return Promise.resolve({ value: self._chunks[idx++], done: false });
        }
        if (!self._closed && self._src && typeof self._src.pull === "function") {
          try {
            var p = self._src.pull(self._ctrl);
            if (p && typeof p.then === "function") {
              return p.then(function() {
                if (self._errored) throw self._errored;
                if (idx < self._chunks.length) return { value: self._chunks[idx++], done: false };
                return { value: undefined, done: true };
              });
            }
          } catch (e) {
            return Promise.reject(e);
          }
          if (idx < self._chunks.length) {
            return Promise.resolve({ value: self._chunks[idx++], done: false });
          }
        }
        return Promise.resolve({ value: undefined, done: true });
      },
      cancel: function(reason) {
        return self.cancel(reason);
      },
      releaseLock: function() {
        self.locked = false;
      }
    };
  };
  ReadableStream.prototype.tee = function() {
    this._drainSync();
    var copy1 = this._chunks.slice(), copy2 = this._chunks.slice();
    return [
      ReadableStream.from(copy1),
      ReadableStream.from(copy2)
    ];
  };
  ReadableStream.prototype.pipeTo = function(writable) {
    var reader = this.getReader();
    var writer = writable.getWriter();
    function pump() {
      return reader.read().then(function(step) {
        if (step.done) {
          reader.releaseLock();
          return writer.close().then(function() { writer.releaseLock(); });
        }
        return writer.write(step.value).then(pump);
      });
    }
    return pump();
  };
  ReadableStream.prototype.pipeThrough = function(transform) {
    if (!transform || !transform.readable || !transform.writable) {
      throw new TypeError("pipeThrough requires a { readable, writable } TransformStream pair");
    }
    this._drainSync();
    var writer = transform.writable.getWriter();
    for (var i = 0; i < this._chunks.length; i++) {
      writer.write(this._chunks[i]);
    }
    writer.close();
    writer.releaseLock();
    return transform.readable;
  };
  globalThis.ReadableStream = ReadableStream;

  function WritableStream(underlyingSink) {
    var self = this;
    this._sink = underlyingSink || null;
    this._closed = false;
    this.locked = false;
    this._ctrl = {
      error: function(e) { self._errored = e || new Error("WritableStream error"); }
    };
    if (this._sink && typeof this._sink.start === "function") {
      this._sink.start(this._ctrl);
    }
  }
  WritableStream.prototype.abort = function(reason) {
    this._closed = true;
    if (this._sink && typeof this._sink.abort === "function") {
      try { return Promise.resolve(this._sink.abort(reason)); } catch (e) { return Promise.reject(e); }
    }
    return Promise.resolve(undefined);
  };
  WritableStream.prototype.close = function() {
    if (this._closed) return Promise.reject(new TypeError("WritableStream is already closed"));
    this._closed = true;
    if (this._sink && typeof this._sink.close === "function") {
      try { return Promise.resolve(this._sink.close()); } catch (e) { return Promise.reject(e); }
    }
    return Promise.resolve(undefined);
  };
  WritableStream.prototype.getWriter = function() {
    if (this.locked) throw new TypeError("WritableStream is already locked to a writer");
    this.locked = true;
    var self = this;
    return {
      ready: Promise.resolve(undefined),
      closed: Promise.resolve(undefined),
      write: function(chunk) {
        if (self._closed) return Promise.reject(new TypeError("WritableStream is closed"));
        if (self._sink && typeof self._sink.write === "function") {
          try {
            return Promise.resolve(self._sink.write(chunk, self._ctrl));
          } catch (e) {
            return Promise.reject(e);
          }
        }
        return Promise.resolve(undefined);
      },
      close: function() {
        return self.close();
      },
      abort: function(reason) {
        return self.abort(reason);
      },
      releaseLock: function() {
        self.locked = false;
      }
    };
  };
  globalThis.WritableStream = WritableStream;

  function TransformStream(transformer) {
    var tr = transformer || {};
    var readable = new ReadableStream();
    var rCtrl = readable._ctrl;
    var tCtrl = {
      desiredSize: 1,
      enqueue: function(chunk) { rCtrl.enqueue(chunk); },
      error: function(e) { rCtrl.error(e); },
      terminate: function() { rCtrl.close(); }
    };
    if (typeof tr.start === "function") {
      tr.start(tCtrl);
    }
    var writable = new WritableStream({
      write: function(chunk) {
        if (typeof tr.transform === "function") {
          return tr.transform(chunk, tCtrl);
        }
        tCtrl.enqueue(chunk);
      },
      close: function() {
        var res = typeof tr.flush === "function" ? tr.flush(tCtrl) : undefined;
        if (res && typeof res.then === "function") {
          return res.then(function() { rCtrl.close(); });
        }
        rCtrl.close();
      },
      abort: function(reason) {
        rCtrl.error(reason);
      }
    });
    this.readable = readable;
    this.writable = writable;
  }
  globalThis.TransformStream = TransformStream;

  function TextEncoderStream() {
    var enc = new globalThis.TextEncoder();
    var ts = new TransformStream({
      transform: function(chunk, ctrl) {
        var s = String(chunk);
        if (s.length > 0) ctrl.enqueue(enc.encode(s));
      }
    });
    this.encoding = "utf-8";
    this.readable = ts.readable;
    this.writable = ts.writable;
  }
  globalThis.TextEncoderStream = TextEncoderStream;

  function TextDecoderStream(label, options) {
    var dec = new globalThis.TextDecoder(label, options);
    var ts = new TransformStream({
      transform: function(chunk, ctrl) {
        var s = dec.decode(chunk);
        if (s.length > 0) ctrl.enqueue(s);
      }
    });
    this.encoding = dec.encoding;
    this.fatal = dec.fatal;
    this.ignoreBOM = dec.ignoreBOM;
    this.readable = ts.readable;
    this.writable = ts.writable;
  }
  globalThis.TextDecoderStream = TextDecoderStream;

  function __normCompFmt(format, ctorName) {
    var f = String(format || "").toLowerCase();
    if (f !== "gzip" && f !== "deflate" && f !== "deflate-raw") {
      throw new TypeError(ctorName + ": Unsupported compression format '" + format + "'");
    }
    return f;
  }

  function CompressionStream(format) {
    var fmt = __normCompFmt(format, "CompressionStream");
    var chunks = [];
    var ts = new TransformStream({
      transform: function(chunk) {
        chunks.push(__slice_toU8(chunk));
      },
      flush: function(ctrl) {
        var input = __slice_concatU8(chunks);
        chunks = [];
        var out = __slice_zlib_compress(input, fmt);
        if (out.byteLength > 0) ctrl.enqueue(out);
      }
    });
    this.readable = ts.readable;
    this.writable = ts.writable;
  }
  globalThis.CompressionStream = CompressionStream;

  function DecompressionStream(format) {
    var fmt = __normCompFmt(format, "DecompressionStream");
    var chunks = [];
    var ts = new TransformStream({
      transform: function(chunk) {
        chunks.push(__slice_toU8(chunk));
      },
      flush: function(ctrl) {
        var input = __slice_concatU8(chunks);
        chunks = [];
        var out = __slice_zlib_decompress(input, fmt);
        if (out.byteLength > 0) ctrl.enqueue(out);
      }
    });
    this.readable = ts.readable;
    this.writable = ts.writable;
  }
  globalThis.DecompressionStream = DecompressionStream;

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
    var u8 = new Uint8Array(this._u8);
    return new ReadableStream({
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
})();

