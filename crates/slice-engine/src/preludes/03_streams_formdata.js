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

  function __rsFinishClose(s) {
    if (s._closed || s._errored) return;
    s._closed = true;
    if (s._waiters) {
      var ws = s._waiters;
      s._waiters = null;
      for (var i = 0; i < ws.length; i++) {
        ws[i].res({ value: undefined, done: true });
      }
    }
    if (s._closeWaiters) {
      var cws = s._closeWaiters;
      s._closeWaiters = null;
      for (var j = 0; j < cws.length; j++) {
        cws[j].res(undefined);
      }
    }
  }

  function __rsError(s, e) {
    if (s._closed || s._errored) return;
    var errVal = arguments.length > 1 ? e : new Error("ReadableStream error");
    s._errored = true;
    s._storedError = errVal;
    s._chunks.length = 0;
    s._head = 0;
    if (s._waiters) {
      var ws = s._waiters;
      s._waiters = null;
      for (var i = 0; i < ws.length; i++) {
        ws[i].rej(errVal);
      }
    }
    if (s._closeWaiters) {
      var cws = s._closeWaiters;
      s._closeWaiters = null;
      for (var j = 0; j < cws.length; j++) {
        cws[j].rej(errVal);
      }
    }
  }

  function __rsShouldCallPull(s) {
    if (!s._started || s._closed || s._closeRequested || s._errored) return false;
    if (!s._src || typeof s._src.pull !== "function") return false;
    if (s.locked && s._waiters && s._waiters.length > 0) return true;
    var desired = s._hwm - (s._chunks.length - s._head);
    return desired > 0;
  }

  function __rsCallPullIfNeeded(s) {
    if (!__rsShouldCallPull(s)) return;
    if (s._pulling) {
      s._pullAgain = true;
      return;
    }
    s._pulling = true;
    var pullPromise;
    try {
      pullPromise = Promise.resolve(s._src.pull(s._ctrl));
    } catch (e) {
      pullPromise = Promise.reject(e);
    }
    pullPromise.then(function() {
      s._pulling = false;
      if (s._pullAgain) {
        s._pullAgain = false;
        __rsCallPullIfNeeded(s);
      }
    }, function(err) {
      s._pulling = false;
      s._pullAgain = false;
      __rsError(s, err);
    });
  }

  function __RSCtrl(stream) {
    this._s = stream;
  }
  Object.defineProperty(__RSCtrl.prototype, "desiredSize", {
    get: function() {
      var s = this._s;
      if (s._errored) return null;
      if (s._closed || s._closeRequested) return 0;
      return s._hwm - (s._chunks.length - s._head);
    },
    configurable: true,
    enumerable: true
  });
  __RSCtrl.prototype.enqueue = function(chunk) {
    var s = this._s;
    if (s._closeRequested || s._closed || s._errored) {
      throw new TypeError("ReadableStream is already closed or errored");
    }
    if (s.locked && s._waiters && s._waiters.length > 0) {
      var w = s._waiters.shift();
      w.res({ value: chunk, done: false });
    } else {
      if (s._chunks.length - s._head >= MAX_STREAM_CHUNKS) {
        throw new RangeError("ReadableStream buffer limit exceeded (" + MAX_STREAM_CHUNKS + " chunks)");
      }
      s._chunks.push(chunk);
    }
    __rsCallPullIfNeeded(s);
  };
  __RSCtrl.prototype.close = function() {
    var s = this._s;
    if (s._closeRequested || s._closed || s._errored) {
      throw new TypeError("ReadableStream is already closed or errored");
    }
    s._closeRequested = true;
    if (s._chunks.length - s._head === 0) {
      __rsFinishClose(s);
    }
  };
  __RSCtrl.prototype.error = function(e) {
    __rsError(this._s, arguments.length > 0 ? e : undefined);
  };
  Object.freeze(__RSCtrl.prototype);

  function __RSReader(stream) {
    this._s = stream;
    this._released = false;
    this._closedPromise = null;
    this._closedWaiter = null;
  }
  Object.defineProperty(__RSReader.prototype, "closed", {
    get: function() {
      if (this._closedPromise) return this._closedPromise;
      var s = this._s;
      if (this._released) {
        this._closedPromise = Promise.reject(new TypeError("ReadableStreamDefaultReader lock has been released"));
        return this._closedPromise;
      }
      if (s._errored) {
        this._closedPromise = Promise.reject(s._storedError);
        return this._closedPromise;
      }
      if (s._closed) {
        this._closedPromise = Promise.resolve(undefined);
        return this._closedPromise;
      }
      var self = this;
      this._closedPromise = new Promise(function(res, rej) {
        var entry = { res: res, rej: rej };
        self._closedWaiter = entry;
        (s._closeWaiters || (s._closeWaiters = [])).push(entry);
      });
      return this._closedPromise;
    },
    configurable: true,
    enumerable: true
  });
  __RSReader.prototype.read = function() {
    if (this._released) {
      return Promise.reject(new TypeError("ReadableStreamDefaultReader lock has been released"));
    }
    var s = this._s;
    if (s._chunks.length > s._head) {
      var chunk = __popStreamChunk(s);
      if (s._closeRequested && s._chunks.length === s._head) {
        __rsFinishClose(s);
      } else {
        __rsCallPullIfNeeded(s);
      }
      return Promise.resolve({ value: chunk, done: false });
    }
    if (s._closed) {
      return Promise.resolve({ value: undefined, done: true });
    }
    if (s._errored) {
      return Promise.reject(s._storedError);
    }
    var readerRef = this;
    var p = new Promise(function(res, rej) {
      (s._waiters || (s._waiters = [])).push({
        reader: readerRef,
        res: res,
        rej: rej
      });
    });
    __rsCallPullIfNeeded(s);
    return p;
  };
  __RSReader.prototype.cancel = function(reason) {
    if (this._released) {
      return Promise.reject(new TypeError("ReadableStreamDefaultReader lock has been released"));
    }
    return this._s._cancelInternal(reason);
  };
  __RSReader.prototype.releaseLock = function() {
    if (this._released) return;
    var s = this._s;
    this._released = true;
    s.locked = false;
    s._reader = null;
    if (s._waiters && s._waiters.length > 0) {
      var relErr = new TypeError("ReadableStreamDefaultReader lock was released while read() was pending");
      var ws = s._waiters;
      s._waiters = null;
      for (var i = 0; i < ws.length; i++) {
        ws[i].rej(relErr);
      }
    }
    if (this._closedWaiter && s._closeWaiters) {
      var idx = s._closeWaiters.indexOf(this._closedWaiter);
      if (idx !== -1) s._closeWaiters.splice(idx, 1);
      if (!s._closed && !s._errored) {
        var lockErr = new TypeError("ReadableStreamDefaultReader lock was released");
        this._closedPromise.catch(function() {});
        this._closedWaiter.rej(lockErr);
      }
      this._closedWaiter = null;
    }
  };
  Object.freeze(__RSReader.prototype);

  function ReadableStream(underlyingSource, strategy) {
    this._chunks = [];
    this._head = 0;
    this._waiters = null;
    this._closeWaiters = null;
    this._started = false;
    this._pulling = false;
    this._pullAgain = false;
    this._closeRequested = false;
    this._closed = false;
    this._errored = false;
    this._storedError = undefined;
    this.locked = false;
    this._reader = null;
    var hwm = (strategy && strategy.highWaterMark !== undefined) ? Number(strategy.highWaterMark) : 1;
    if (Number.isNaN(hwm) || hwm < 0) {
      throw new RangeError("Invalid highWaterMark");
    }
    this._hwm = hwm;
    this._src = underlyingSource || null;
    var ctrl = new __RSCtrl(this);
    this._ctrl = ctrl;
    this._startPromise = null;
    var self = this;
    var startResult;
    if (this._src && typeof this._src.start === "function") {
      startResult = this._src.start(ctrl);
    }
    if (startResult && typeof startResult.then === "function") {
      this._startPromise = Promise.resolve(startResult).then(function() {
        self._startPromise = null;
        self._started = true;
        __rsCallPullIfNeeded(self);
      }, function(err) {
        self._startPromise = null;
        __rsError(self, err);
      });
    } else {
      Promise.resolve(undefined).then(function() {
        self._started = true;
        __rsCallPullIfNeeded(self);
      });
    }
  }
  ReadableStream.from = function(iterable) {
    if (iterable === null || iterable === undefined) {
      throw new TypeError("ReadableStream.from requires an iterable or async iterable");
    }
    if (Array.isArray(iterable)) {
      return new ReadableStream({
        start: function(c) {
          for (var i = 0; i < iterable.length; i++) c.enqueue(iterable[i]);
          c.close();
        }
      });
    }
    var asyncSym = typeof Symbol !== "undefined" ? Symbol.asyncIterator : null;
    if (asyncSym && typeof iterable[asyncSym] === "function") {
      var ait = iterable[asyncSym]();
      return new ReadableStream({
        pull: function(c) {
          return Promise.resolve(ait.next()).then(function(step) {
            if (step.done) c.close();
            else c.enqueue(step.value);
          });
        }
      });
    }
    if (typeof iterable[Symbol.iterator] === "function") {
      var it = iterable[Symbol.iterator]();
      return new ReadableStream({
        pull: function(c) {
          var step = it.next();
          if (step.done) c.close();
          else c.enqueue(step.value);
        }
      });
    }
    throw new TypeError("ReadableStream.from argument is not iterable");
  };
  ReadableStream.prototype._isAsyncPending = function() {
    if (this._closed || this._errored) return false;
    if (this._startPromise || this._pulling) return true;
    return Boolean(this._src && typeof this._src.pull === "function");
  };
  ReadableStream.prototype._drainSync = function() {
    var guard = 0;
    while (!this._closed && !this._closeRequested && !this._errored && !this._pulling && this._src && typeof this._src.pull === "function" && guard < MAX_STREAM_CHUNKS) {
      guard++;
      var before = this._chunks.length;
      var r = this._src.pull(this._ctrl);
      if (r && typeof r.then === "function") break;
      if (this._chunks.length === before && !this._closed && !this._closeRequested) break;
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
  ReadableStream.prototype._cancelInternal = function(reason) {
    if (this._closed) return Promise.resolve(undefined);
    if (this._errored) return Promise.reject(this._storedError);
    this._closeRequested = true;
    this._chunks.length = 0;
    this._head = 0;
    __rsFinishClose(this);
    if (this._src && typeof this._src.cancel === "function") {
      try {
        return Promise.resolve(this._src.cancel(reason)).then(function() { return undefined; });
      } catch (e) {
        return Promise.reject(e);
      }
    }
    return Promise.resolve(undefined);
  };
  ReadableStream.prototype.cancel = function(reason) {
    if (this.locked) {
      return Promise.reject(new TypeError("Cannot cancel a locked ReadableStream"));
    }
    return this._cancelInternal(reason);
  };
  ReadableStream.prototype.getReader = function() {
    if (this.locked) throw new TypeError("ReadableStream is already locked to a reader");
    this.locked = true;
    var r = new __RSReader(this);
    this._reader = r;
    return r;
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
    var reader = this.getReader();
    var reading = false;
    var readAgain = false;
    var canceled1 = false;
    var canceled2 = false;
    var reason1;
    var reason2;
    var branch1, branch2;
    var resolveCancel;
    var cancelPromise = new Promise(function(res) { resolveCancel = res; });

    function pullAlgorithm() {
      if (reading) {
        readAgain = true;
        return Promise.resolve(undefined);
      }
      reading = true;
      reader.read().then(function(step) {
        reading = false;
        if (step.done) {
          if (!canceled1) {
            try { branch1._ctrl.close(); } catch (_e1) {}
          }
          if (!canceled2) {
            try { branch2._ctrl.close(); } catch (_e2) {}
          }
          if (!canceled1 || !canceled2) resolveCancel(undefined);
          return;
        }
        var val1 = step.value;
        var val2 = step.value;
        if (!canceled1 && !canceled2 && val1 instanceof Uint8Array) {
          val2 = new Uint8Array(val1);
        }
        if (!canceled1) branch1._ctrl.enqueue(val1);
        if (!canceled2) branch2._ctrl.enqueue(val2);
        if (readAgain) {
          readAgain = false;
          pullAlgorithm();
        }
      }, function(err) {
        reading = false;
        branch1._ctrl.error(err);
        branch2._ctrl.error(err);
        if (!canceled1 || !canceled2) resolveCancel(undefined);
      });
      return Promise.resolve(undefined);
    }

    branch1 = new ReadableStream({
      pull: pullAlgorithm,
      cancel: function(r) {
        canceled1 = true;
        reason1 = r;
        if (canceled2) {
          resolveCancel(reader.cancel([reason1, reason2]));
        }
        return cancelPromise;
      }
    });
    branch2 = new ReadableStream({
      pull: pullAlgorithm,
      cancel: function(r) {
        canceled2 = true;
        reason2 = r;
        if (canceled1) {
          resolveCancel(reader.cancel([reason1, reason2]));
        }
        return cancelPromise;
      }
    });
    return [branch1, branch2];
  };
  ReadableStream.prototype.pipeTo = function(writable, options) {
    var opts = options || {};
    var preventClose = Boolean(opts.preventClose);
    var preventAbort = Boolean(opts.preventAbort);
    var preventCancel = Boolean(opts.preventCancel);
    var sig = opts.signal || null;
    if (sig && sig.aborted) {
      var abortErr = sig.reason !== undefined ? sig.reason : new Error("Aborted");
      var actions = [];
      if (!preventAbort) actions.push(writable.abort(abortErr));
      if (!preventCancel) actions.push(this.cancel(abortErr));
      return Promise.all(actions).then(function() { throw abortErr; });
    }
    var reader = this.getReader();
    var writer = writable.getWriter();
    function pump() {
      return reader.read().then(function(step) {
        if (step.done) {
          reader.releaseLock();
          if (preventClose) {
            writer.releaseLock();
            return undefined;
          }
          return writer.close().then(function() { writer.releaseLock(); });
        }
        return writer.write(step.value).then(pump);
      }, function(err) {
        reader.releaseLock();
        if (!preventAbort) {
          return writer.abort(err).then(function() {
            writer.releaseLock();
            throw err;
          }, function() {
            writer.releaseLock();
            throw err;
          });
        }
        writer.releaseLock();
        throw err;
      });
    }
    return pump();
  };
  ReadableStream.prototype.pipeThrough = function(transform, options) {
    if (!transform || !transform.readable || !transform.writable) {
      throw new TypeError("pipeThrough requires a { readable, writable } TransformStream pair");
    }
    this.pipeTo(transform.writable, options).catch(function() {});
    return transform.readable;
  };
  ReadableStream.prototype._drainToBody = function(self) {
    var st = this;
    self._stream = null;
    if (st._closed && !st._startPromise && !st._pulling) {
      self._u8 = st._toBytes();
      self._body = null;
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
            self._u8 = __slice_concatU8(u8Parts);
            self._body = null;
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
