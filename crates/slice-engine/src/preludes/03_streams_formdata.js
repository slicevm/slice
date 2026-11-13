// Slice (Nexus-JS) Lazy Prelude Shard 3: WHATWG `ReadableStream` (`03_streams_formdata.js`)
(function() {
  var MAX_STREAM_CHUNKS = 4096;

  function __popStreamChunk(s) {
    var v = s._chunks[s._head++];
    if (s._head === s._chunks.length) {
      s._chunks.length = 0;
      s._head = 0;
    } else if (s._head >= 64 && s._head * 2 >= s._chunks.length) {
      s._chunks = s._chunks.slice(s._head);
      s._head = 0;
    }
    return v;
  }

  function __RSCtrl(stream) {
    this._s = stream;
    this.desiredSize = 1;
  }
  __RSCtrl.prototype.enqueue = function(chunk) {
    var s = this._s;
    if (s._closed) throw new TypeError("ReadableStream is already closed");
    if (s._chunks.length - s._head >= MAX_STREAM_CHUNKS) {
      throw new RangeError("ReadableStream buffer limit exceeded (" + MAX_STREAM_CHUNKS + " chunks)");
    }
    s._chunks.push(chunk);
    if (s._waiters && s._waiters.length > 0) {
      var w = s._waiters.shift();
      w.res({ value: __popStreamChunk(s), done: false });
    }
  };
  __RSCtrl.prototype.close = function() {
    var s = this._s;
    s._closed = true;
    if (s._waiters) {
      while (s._waiters.length > 0) {
        s._waiters.shift().res({ value: undefined, done: true });
      }
    }
    if (s._closeWaiters) {
      while (s._closeWaiters.length > 0) {
        s._closeWaiters.shift().res(undefined);
      }
    }
  };
  __RSCtrl.prototype.error = function(e) {
    var s = this._s;
    s._errored = e || new Error("ReadableStream error");
    s._closed = true;
    if (s._waiters) {
      while (s._waiters.length > 0) {
        s._waiters.shift().rej(s._errored);
      }
    }
    if (s._closeWaiters) {
      while (s._closeWaiters.length > 0) {
        s._closeWaiters.shift().rej(s._errored);
      }
    }
  };
  Object.freeze(__RSCtrl.prototype);

  function __RSReader(stream) {
    this._s = stream;
    this._released = false;
    this.closed = Promise.resolve(undefined);
  }
  __RSReader.prototype.read = function() {
    if (this._released) return Promise.reject(new TypeError("ReadableStreamDefaultReader lock has been released"));
    var s = this._s;
    if (s._errored) return Promise.reject(s._errored);
    if (s._head < s._chunks.length) {
      return Promise.resolve({ value: __popStreamChunk(s), done: false });
    }
    if (!s._closed && s._src && typeof s._src.pull === "function") {
      try {
        var p = s._src.pull(s._ctrl);
        if (p && typeof p.then === "function") {
          return p.then(function() {
            if (s._errored) throw s._errored;
            if (s._head < s._chunks.length) return { value: __popStreamChunk(s), done: false };
            return { value: undefined, done: true };
          });
        }
      } catch (e) {
        return Promise.reject(e);
      }
      if (s._head < s._chunks.length) {
        return Promise.resolve({ value: __popStreamChunk(s), done: false });
      }
    }
    if (!s._closed) {
      var readerRef = this;
      return new Promise(function(res, rej) {
        (s._waiters || (s._waiters = [])).push({
          reader: readerRef,
          res: res,
          rej: rej
        });
      });
    }
    return Promise.resolve({ value: undefined, done: true });
  };
  __RSReader.prototype.cancel = function(reason) {
    if (this._released) return Promise.reject(new TypeError("ReadableStreamDefaultReader lock has been released"));
    return this._s.cancel(reason);
  };
  __RSReader.prototype.releaseLock = function() {
    if (this._released) return;
    this._released = true;
    this._s.locked = false;
  };
  Object.freeze(__RSReader.prototype);

  function ReadableStream(underlyingSource) {
    this._chunks = [];
    this._head = 0;
    this._waiters = null;
    this._closeWaiters = null;
    this._closed = false;
    this._errored = null;
    this.locked = false;
    this._src = underlyingSource || null;
    var ctrl = new __RSCtrl(this);
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
    for (var i = this._head; i < this._chunks.length; i++) {
      var c = this._chunks[i];
      if (typeof c === "string") out += c;
      else if (c instanceof Uint8Array || c instanceof ArrayBuffer || (c && c.buffer instanceof ArrayBuffer)) {
        if (!globalThis.TextDecoder) __slice_ensure_shard(2);
        out += __slice_text_decode(c, "utf-8", false, false);
      } else if (c !== undefined && c !== null) {
        out += String(c);
      }
    }
    this._chunks = [];
    this._head = 0;
    return out;
  };
  ReadableStream.prototype._toBytes = function() {
    this._drainSync();
    var u8s = [];
    for (var i = this._head; i < this._chunks.length; i++) u8s.push(__slice_toU8(this._chunks[i]));
    this._chunks = [];
    this._head = 0;
    return __slice_concatU8(u8s);
  };
  ReadableStream.prototype.cancel = function(reason) {
    this._closed = true;
    this._chunks = [];
    this._head = 0;
    if (this._src && typeof this._src.cancel === "function") {
      try { return Promise.resolve(this._src.cancel(reason)); } catch (e) { return Promise.reject(e); }
    }
    return Promise.resolve(undefined);
  };
  ReadableStream.prototype.getReader = function() {
    if (this.locked) throw new TypeError("ReadableStream is already locked to a reader");
    this.locked = true;
    return new __RSReader(this);
  };
  ReadableStream.prototype.values = function(opts) {
    var preventCancel = Boolean(opts && opts.preventCancel);
    var reader = this.getReader();
    return {
      next: function() {
        return reader.read().then(function(step) {
          if (step.done) reader.releaseLock();
          return step;
        }, function(e) {
          reader.releaseLock();
          throw e;
        });
      },
      return: function(val) {
        if (!preventCancel) {
          return reader.cancel(val).then(function() {
            reader.releaseLock();
            return { value: val, done: true };
          });
        }
        reader.releaseLock();
        return Promise.resolve({ value: val, done: true });
      },
      [Symbol.asyncIterator]: function() { return this; }
    };
  };
  if (typeof Symbol !== "undefined" && Symbol.asyncIterator) {
    ReadableStream.prototype[Symbol.asyncIterator] = ReadableStream.prototype.values;
  }
  ReadableStream.prototype.tee = function() {
    this._drainSync();
    var copy1 = this._chunks.slice(this._head), copy2 = copy1.slice();
    this._chunks = [];
    this._head = 0;
    this._closed = true;
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
    for (var i = this._head; i < this._chunks.length; i++) {
      writer.write(this._chunks[i]);
    }
    this._chunks = [];
    this._head = 0;
    writer.close();
    writer.releaseLock();
    return transform.readable;
  };
  ReadableStream.prototype._drainToBody = function(self) {
    var st = this;
    self._stream = null;
    if (st._closed && !st._startPromise) {
      var u8Sync = st._toBytes();
      self._u8 = u8Sync;
      if (!globalThis.TextDecoder) __slice_ensure_shard(2);
      self._body = __slice_text_decode(u8Sync, "utf-8", false, false);
      return Promise.resolve(self);
    }
    var startWait = st._startPromise ? st._startPromise : Promise.resolve();
    return startWait.then(function() {
      var reader = st.getReader();
      var u8Parts = [];
      function pump() {
        return reader.read().then(function(step) {
          if (step.done) {
            reader.releaseLock();
            var fullU8 = __slice_concatU8(u8Parts);
            self._u8 = fullU8;
            if (!globalThis.TextDecoder) __slice_ensure_shard(2);
            self._body = __slice_text_decode(fullU8, "utf-8", false, false);
            return self;
          }
          u8Parts.push(__slice_toU8(step.value));
          return pump();
        });
      }
      return pump();
    });
  };
  globalThis.ReadableStream = ReadableStream;
})();
