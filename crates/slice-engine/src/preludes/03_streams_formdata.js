// Slice (Nexus-JS) Lazy Prelude Shard 3: WHATWG `ReadableStream` (`03_streams_formdata.js`)
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
    this._startPromise = null;
    if (this._src && typeof this._src.start === "function") {
      var sp = this._src.start(ctrl);
      if (sp && typeof sp.then === "function") this._startPromise = sp;
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
        if (!globalThis.TextDecoder) __slice_ensure_shard(2);
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
})();
