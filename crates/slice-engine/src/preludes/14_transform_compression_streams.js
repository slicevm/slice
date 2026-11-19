// Slice (Nexus-JS) Lazy Prelude Shard 14: WHATWG `WritableStream`, `TransformStream`, `TextEncoderStream`, `TextDecoderStream`, `CompressionStream`, `DecompressionStream` (`14_transform_compression_streams.js`)
(function() {
  if (!globalThis.ReadableStream) __slice_ensure_shard(3);
  if (!globalThis.TextEncoder) __slice_ensure_shard(2);
  var ReadableStream = globalThis.ReadableStream;

  function __wsError(ws, err) {
    if (ws._state === "closed" || ws._state === "errored") return;
    ws._state = "errored";
    ws._storedError = err;
    if (ws._closeWaiters) {
      var cws = ws._closeWaiters;
      ws._closeWaiters = null;
      for (var i = 0; i < cws.length; i++) {
        cws[i].rej(err);
      }
    }
  }

  function WritableStream(underlyingSink, strategy) {
    var self = this;
    this._sink = underlyingSink || null;
    this._state = "writable";
    this._storedError = undefined;
    this.locked = false;
    this._writer = null;
    this._closeWaiters = null;
    this._inFlightWrite = Promise.resolve(undefined);
    var hwm = (strategy && strategy.highWaterMark !== undefined) ? Number(strategy.highWaterMark) : 1;
    if (Number.isNaN(hwm) || hwm < 0) {
      throw new RangeError("Invalid highWaterMark");
    }
    this._hwm = hwm;
    this._ctrl = {
      error: function(e) {
        __wsError(self, arguments.length > 0 ? e : undefined);
      }
    };
    if (this._sink && typeof this._sink.start === "function") {
      var startRes = this._sink.start(this._ctrl);
      if (startRes && typeof startRes.then === "function") {
        Promise.resolve(startRes).then(undefined, function(err) {
          __wsError(self, err);
        });
      }
    }
  }
  WritableStream.prototype._abortInternal = function(reason) {
    if (this._state === "closed" || this._state === "errored") {
      return Promise.resolve(undefined);
    }
    __wsError(this, reason);
    if (this._sink && typeof this._sink.abort === "function") {
      try {
        return Promise.resolve(this._sink.abort(reason)).then(function() { return undefined; });
      } catch (e) {
        return Promise.reject(e);
      }
    }
    return Promise.resolve(undefined);
  };
  WritableStream.prototype._closeInternal = function() {
    if (this._state === "closed" || this._state === "closing") {
      return Promise.reject(new TypeError("WritableStream is already closed or closing"));
    }
    if (this._state === "errored") {
      return Promise.reject(this._storedError);
    }
    var self = this;
    this._state = "closing";
    var p = this._inFlightWrite.then(function() {
      if (self._state === "errored") throw self._storedError;
      if (self._sink && typeof self._sink.close === "function") {
        return self._sink.close();
      }
      return undefined;
    }).then(function() {
      if (self._state !== "errored") {
        self._state = "closed";
        if (self._closeWaiters) {
          var cws = self._closeWaiters;
          self._closeWaiters = null;
          for (var i = 0; i < cws.length; i++) cws[i].res(undefined);
        }
      }
      return undefined;
    }, function(err) {
      __wsError(self, err);
      throw err;
    });
    this._inFlightWrite = p.then(undefined, function() {});
    return p;
  };
  WritableStream.prototype.abort = function(reason) {
    if (!(this instanceof WritableStream)) {
      return Promise.reject(new TypeError("Invalid WritableStream receiver"));
    }
    if (this.locked) {
      return Promise.reject(new TypeError("Cannot abort a locked WritableStream"));
    }
    return this._abortInternal(reason);
  };
  WritableStream.prototype.close = function() {
    if (!(this instanceof WritableStream)) {
      return Promise.reject(new TypeError("Invalid WritableStream receiver"));
    }
    if (this.locked) {
      return Promise.reject(new TypeError("Cannot close a locked WritableStream"));
    }
    return this._closeInternal();
  };
  WritableStream.prototype.getWriter = function() {
    if (this.locked) throw new TypeError("WritableStream is already locked to a writer");
    this.locked = true;
    var self = this;
    var released = false;
    var closedPromise = null;
    var closedWaiter = null;
    var writer = {
      get desiredSize() {
        if (released) throw new TypeError("WritableStreamDefaultWriter lock has been released");
        if (self._state === "errored") return null;
        if (self._state === "closed" || self._state === "closing") return 0;
        return self._hwm;
      },
      get ready() {
        if (released) return Promise.reject(new TypeError("WritableStreamDefaultWriter lock has been released"));
        if (self._state === "errored") return Promise.reject(self._storedError);
        return Promise.resolve(undefined);
      },
      get closed() {
        if (closedPromise) return closedPromise;
        if (released) {
          closedPromise = Promise.reject(new TypeError("WritableStreamDefaultWriter lock has been released"));
          return closedPromise;
        }
        if (self._state === "errored") {
          closedPromise = Promise.reject(self._storedError);
          return closedPromise;
        }
        if (self._state === "closed") {
          closedPromise = Promise.resolve(undefined);
          return closedPromise;
        }
        closedPromise = new Promise(function(res, rej) {
          closedWaiter = { res: res, rej: rej };
          (self._closeWaiters || (self._closeWaiters = [])).push(closedWaiter);
        });
        return closedPromise;
      },
      write: function(chunk) {
        if (released) {
          return Promise.reject(new TypeError("WritableStreamDefaultWriter lock has been released"));
        }
        if (self._state === "errored") {
          return Promise.reject(self._storedError);
        }
        if (self._state === "closed" || self._state === "closing") {
          return Promise.reject(new TypeError("Cannot write to a closed or closing WritableStream"));
        }
        var p = self._inFlightWrite.then(function() {
          if (self._state === "errored") throw self._storedError;
          if (self._sink && typeof self._sink.write === "function") {
            return self._sink.write(chunk, self._ctrl);
          }
          return undefined;
        }).then(function() {
          return undefined;
        }, function(err) {
          __wsError(self, err);
          throw err;
        });
        self._inFlightWrite = p.then(undefined, function() {});
        return p;
      },
      close: function() {
        if (released) {
          return Promise.reject(new TypeError("WritableStreamDefaultWriter lock has been released"));
        }
        return self._closeInternal();
      },
      abort: function(reason) {
        if (released) {
          return Promise.reject(new TypeError("WritableStreamDefaultWriter lock has been released"));
        }
        return self._abortInternal(reason);
      },
      releaseLock: function() {
        if (released) return;
        released = true;
        self.locked = false;
        self._writer = null;
        if (closedWaiter && self._closeWaiters) {
          var idx = self._closeWaiters.indexOf(closedWaiter);
          if (idx !== -1) self._closeWaiters.splice(idx, 1);
          if (self._state !== "closed" && self._state !== "errored") {
            var lockErr = new TypeError("WritableStreamDefaultWriter lock was released");
            closedPromise.catch(function() {});
            closedWaiter.rej(lockErr);
          }
          closedWaiter = null;
        }
      }
    };
    this._writer = writer;
    return writer;
  };
  globalThis.WritableStream = WritableStream;

  function TransformStream(transformer) {
    var tr = transformer || {};
    var readable = new ReadableStream();
    var rCtrl = readable._ctrl;
    var tCtrl = {
      desiredSize: 1,
      enqueue: function(chunk) { rCtrl.enqueue(chunk); },
      error: function(e) { rCtrl.error(arguments.length > 0 ? e : undefined); },
      terminate: function() {
        if (!readable._closeRequested && !readable._closed && !readable._errored) {
          rCtrl.close();
        }
      }
    };
    if (typeof tr.start === "function") {
      var startRes = tr.start(tCtrl);
      if (startRes && typeof startRes.then === "function") {
        Promise.resolve(startRes).then(undefined, function(err) {
          rCtrl.error(err);
        });
      }
    }
    var writable = new WritableStream({
      write: function(chunk) {
        if (typeof tr.transform === "function") {
          var p;
          try {
            p = Promise.resolve(tr.transform(chunk, tCtrl));
          } catch (e) {
            rCtrl.error(e);
            return Promise.reject(e);
          }
          return p.then(undefined, function(e) {
            rCtrl.error(e);
            throw e;
          });
        }
        try {
          tCtrl.enqueue(chunk);
          return Promise.resolve(undefined);
        } catch (e2) {
          rCtrl.error(e2);
          return Promise.reject(e2);
        }
      },
      close: function() {
        if (typeof tr.flush === "function") {
          var p;
          try {
            p = Promise.resolve(tr.flush(tCtrl));
          } catch (e) {
            rCtrl.error(e);
            return Promise.reject(e);
          }
          return p.then(function() {
            if (!readable._closeRequested && !readable._closed && !readable._errored) {
              rCtrl.close();
            }
          }, function(e) {
            rCtrl.error(e);
            throw e;
          });
        }
        if (!readable._closeRequested && !readable._closed && !readable._errored) {
          rCtrl.close();
        }
        return Promise.resolve(undefined);
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
        var s = dec.decode(chunk, { stream: true });
        if (s.length > 0) ctrl.enqueue(s);
      },
      flush: function(ctrl) {
        var rem = dec.decode();
        if (rem.length > 0) ctrl.enqueue(rem);
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

