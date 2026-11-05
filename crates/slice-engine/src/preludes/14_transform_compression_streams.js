// Slice (Nexus-JS) Lazy Prelude Shard 14: WHATWG `WritableStream`, `TransformStream`, `TextEncoderStream`, `TextDecoderStream`, `CompressionStream`, `DecompressionStream` (`14_transform_compression_streams.js`)
(function() {
  if (!globalThis.ReadableStream) __slice_ensure_shard(3);
  if (!globalThis.TextEncoder) __slice_ensure_shard(2);
  var ReadableStream = globalThis.ReadableStream;

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
})();

