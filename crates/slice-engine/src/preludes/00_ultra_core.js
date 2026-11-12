// Slice (Nexus-JS) Ultra-Core Bootstrap Prelude (`00_ultra_core.js`)
// Evaluated eagerly on realm creation; registers self-replacing lazy getters (`__defLazy`)
// for Shards 1..=7 so unused WinterTC/Node.js subsystems incur 0 bytes of parse/eval cost.
(function() {
  globalThis.__export = function(t, a) {
    for (var k in a) Object.defineProperty(t, k, { get: a[k], enumerable: true });
  };
  globalThis.__toCommonJS = function(m) {
    return m;
  };
  globalThis.__shMask = 0;
  globalThis.__shardProps = {};
  var __lazyInited = false;
  var __sesReady = false, __sesP = null, __sesN = null, __sesK = null, __sesL = null;
  var __wu = [];
  globalThis.__slice_mark_lazy_inited = function() { __lazyInited = true; };
  function __loadShard(id) {
    if (!__lazyInited) {
      if (
        (id === 1 && globalThis.URL) ||
        (id === 2 && globalThis.TextEncoder) ||
        (id === 3 && globalThis.ReadableStream) ||
        (id === 4 && globalThis.crypto) ||
        (id === 5 && globalThis.Buffer) ||
        (id === 7 && globalThis.WebSocket) ||
        (id === 8 && globalThis.Worker) ||
        (id === 9 && globalThis.WebAssembly) ||
        (id === 10 && globalThis.fetch) ||
        (id === 11 && globalThis.Request) ||
        (id === 12 && globalThis.URLPattern) ||
        (id === 13 && globalThis.EventTarget) ||
        (id === 14 && globalThis.TransformStream) ||
        (id === 15 && globalThis.FormData) ||
        (id === 16 && globalThis.structuredClone) ||
        (id === 17 && globalThis.Buffer && globalThis.Buffer.prototype._hasBinRW) ||
        (id === 18 && globalThis.Intl) ||
        (id === 29 && globalThis.navigator)
      ) {
        globalThis.__shMask |= (1 << id);
        return;
      }
    }
    if ((id === 7 || id === 8) && (globalThis.__shMask & (1 << 13)) === 0 && (__lazyInited || !globalThis.EventTarget)) {
      __loadShard(13);
    }
    if (id === 8 && (globalThis.__shMask & 4) === 0 && (__lazyInited || !globalThis.TextEncoder)) {
      __loadShard(2);
    }
    if (id === 10 && (globalThis.__shMask & (1 << 11)) === 0 && (__lazyInited || !globalThis.Request)) {
      __loadShard(11);
    }
    if (id === 14 && (globalThis.__shMask & 8) === 0 && (__lazyInited || !globalThis.ReadableStream)) {
      __loadShard(3);
    }
    if (id === 17 && (globalThis.__shMask & 32) === 0 && (__lazyInited || !globalThis.Buffer)) {
      __loadShard(5);
    }
    var bit = 1 << id;
    if ((globalThis.__shMask & bit) === 0) {
      globalThis.__shMask |= bit;
      var ns = globalThis.__shardProps[id];
      if (ns) {
        for (var i = 0; i < ns.length; i++) {
          Object.defineProperty(globalThis, ns[i], {
            value: undefined,
            writable: true,
            enumerable: false,
            configurable: true
          });
        }
      }
      __slice_lazy_prelude(id);
      if (id === 18 && __sesReady) {
        __sesK[3] = Reflect.ownKeys(String.prototype);
        __sesL[3] = __sesK[3].length;
      }
    }
  }
  Object.defineProperty(globalThis, "__slice_ensure_shard", {
    value: __loadShard,
    writable: false,
    enumerable: false,
    configurable: false
  });

  globalThis.__slice_dispatch_fetch = function(h) {
    if (!__slice_http_is_active()) return;
    __loadShard(11);
    return globalThis.__slice_dispatch_fetch(h);
  };

  function __fastConsoleLog(a) {
    if (arguments.length === 1 && (typeof a !== "object" || a === null)) {
      __slice_console_log(a);
      return;
    }
    __loadShard(6);
    return globalThis.console.log.apply(globalThis.console, arguments);
  }

  globalThis.console = {
    log: __fastConsoleLog,
    info: __fastConsoleLog,
    warn: __fastConsoleLog,
    error: __fastConsoleLog,
    debug: __fastConsoleLog
  };

  globalThis.print = __slice_console_log;
  globalThis.env = function(k) {
    return __slice_env_get(String(k));
  };

  var __pExitCode = undefined;
  var __argvCache = null;
  function __pLazy(m) {
    return function() {
      __slice_process_ctl(0);
      __loadShard(29);
      return globalThis.process[m].apply(globalThis.process, arguments);
    };
  }

  var __ProcProto = {
    on: __pLazy("on"),
    once: __pLazy("once"),
    off: __pLazy("off"),
    addListener: __pLazy("addListener"),
    removeListener: __pLazy("removeListener"),
    removeAllListeners: __pLazy("removeAllListeners"),
    listeners: __pLazy("listeners"),
    listenerCount: __pLazy("listenerCount"),
    emitWarning: __pLazy("emitWarning")
  };

  var __proc = Object.create(__ProcProto);
  __proc.pid = 1;
  __proc.env = { NODE_ENV: "production" };
  __proc.cwd = function() { return "."; };
  __proc._exited = false;
  __proc._resetExitCode = function() {
    __pExitCode = undefined;
    __argvCache = null;
    __proc._exited = false;
  };
  __proc.version = "v24.0.0";
  __proc.versions = { node: "24.0.0" };
  __proc.exit = function(code) {
    __ntQ.length = 0;
    __qmQ.length = 0;
    __mtS = false;
    __slice_process_ctl(2, code);
  };
  Object.defineProperties(__proc, {
    platform: { get: __slice_os_platform, enumerable: true, configurable: true },
    arch: { get: __slice_os_arch, enumerable: true, configurable: true },
    exitCode: {
      get: function() { return __pExitCode; },
      set: function(v) {
        __pExitCode = v;
        if (v !== undefined) __slice_process_ctl(0);
      },
      enumerable: true,
      configurable: true
    },
    argv: {
      get: function() {
        if (!__argvCache) __argvCache = __slice_process_ctl(1);
        return __argvCache;
      },
      set: function(v) {
        __slice_process_ctl(0);
        __argvCache = v;
      },
      enumerable: true,
      configurable: true
    }
  });
  globalThis.process = __proc;

  function __syncProcessEnv(load) {
    if (!load) {
      globalThis.process.env = { NODE_ENV: "production" };
      return;
    }
    var m = JSON.parse(__slice_env_json());
    m.NODE_ENV = m.NODE_ENV || "production";
    globalThis.process.env = m;
  }

  var __orand = Math.random;
  var __onow = Date.now;
  var __perfNowNative = __slice_perf_now;
  globalThis.performance = {
    now: __perfNowNative
  };
  function __setDetMode(on) {
    if (on) {
      Math.random = function() {
        var v = __slice_det_random();
        return v >= 0 ? v : __orand();
      };
      Date.now = function() {
        var v = __slice_det_now();
        return v >= 0 ? v : __onow();
      };
      globalThis.performance.now = function() {
        var v = __slice_det_now();
        return (v >= 0 ? v : __onow()) - 1700000000000;
      };
    } else {
      Math.random = __orand;
      Date.now = __onow;
      globalThis.performance.now = __perfNowNative;
    }
  }

  // `process.nextTick` & `queueMicrotask`:
  // Speculative On-Demand `Promise.prototype.then` Hook:
  // By default, `Promise.prototype.then` stays bound to Boa's native Rust `__origThen` so
  // `await` and `.then()` allocate 0 JS wrapper closures in scripts that do not use `nextTick`.
  // When a script uses `process.nextTick` (or calls `__slice_enable_next_tick_hook()`),
  // `Promise.prototype.then` is hooked to `__wrappedThen` so `nextTick` callbacks always
  // drain before any `.then()` or `queueMicrotask` handler in the same turn.
  var __ntQ = [];
  var __qmQ = [];
  var __mtS = false;
  var __ntHooked = false;
  var __alsWrap = null;
  var __origThen = Promise.prototype.then;

  function __wrappedThen(onF, onR) {
    var wF = typeof onF === "function" ? function(v) {
      if (__ntQ.length > 0) {
        while (__ntQ.length > 0) {
          var n = __ntQ.shift();
          if (n.a) n.f.apply(undefined, n.a);
          else n.f();
        }
      }
      return onF(v);
    } : onF;
    var wR = typeof onR === "function" ? function(e) {
      if (__ntQ.length > 0) {
        while (__ntQ.length > 0) {
          var n = __ntQ.shift();
          if (n.a) n.f.apply(undefined, n.a);
          else n.f();
        }
      }
      return onR(e);
    } : onR;
    return __origThen.call(this, wF, wR);
  }

  function __setNextTickHook(enable) {
    if (enable && !__ntHooked) {
      __ntHooked = true;
      Promise.prototype.then = __wrappedThen;
    } else if (!enable && __ntHooked) {
      __ntQ.length = 0;
      __ntHooked = false;
      Promise.prototype.then = __origThen;
    }
  }

  function __resetMt() {
    __ntQ.length = 0;
    __qmQ.length = 0;
    __mtS = false;
  }

  function __ctl(op, flag) {
    if (op === 0) {
      __ntQ.length = 0;
      __qmQ.length = 0;
      __mtS = false;
    } else if (op === 1) {
      __setDetMode(flag);
    } else if (op === 2) {
      __syncProcessEnv(flag);
    } else if (op === 3) {
      __setNextTickHook(flag);
    } else if (op === 4) {
      __wu.length = 0;
    } else if (op === 5) {
      var s = globalThis.__slice_active_http_server;
      if (s && typeof s._dispatchActiveHttpBridge === "function") {
        s._dispatchActiveHttpBridge();
      }
    }
  }

  globalThis.__slice_set_als_wrap = function(wrapFn) {
    __alsWrap = wrapFn || null;
  };

  function __schedMt() {
    if (__mtS) return;
    __mtS = true;
    __origThen.call(Promise.resolve(), __drainMt);
  }

  function __drainMt() {
    while (__ntQ.length > 0 || __qmQ.length > 0) {
      while (__ntQ.length > 0) {
        var n = __ntQ.shift();
        if (n.a) n.f.apply(undefined, n.a);
        else n.f();
      }
      if (__qmQ.length > 0) {
        var q = __qmQ.shift();
        q();
      }
    }
    __mtS = false;
    if (__ntQ.length > 0 || __qmQ.length > 0) __schedMt();
  }

  globalThis.process.nextTick = function(cb) {
    if (typeof cb !== "function") {
      throw new TypeError("process.nextTick requires a callback function");
    }
    if (!__ntHooked) {
      __ntHooked = true;
      Promise.prototype.then = __wrappedThen;
    }
    var a = undefined;
    if (arguments.length > 1) {
      a = [];
      for (var i = 1; i < arguments.length; i++) a.push(arguments[i]);
    }
    __ntQ.push({ f: __alsWrap ? __alsWrap(cb) : cb, a: a });
    __schedMt();
  };

  globalThis.queueMicrotask = function(cb) {
    if (typeof cb !== "function") {
      throw new TypeError("queueMicrotask requires a callback function");
    }
    __qmQ.push(__alsWrap ? __alsWrap(cb) : cb);
    __schedMt();
  };

  function __Timeout(id) {
    this._id = id;
    this._refed = true;
  }
  __Timeout.prototype.unref = function() {
    this._refed = false;
    __slice_timer_ctl(this._id, 2);
    return this;
  };
  __Timeout.prototype.ref = function() {
    this._refed = true;
    __slice_timer_ctl(this._id, 1);
    return this;
  };
  __Timeout.prototype.hasRef = function() {
    return this._refed;
  };
  __Timeout.prototype.refresh = function() {
    __slice_timer_ctl(this._id, 3);
    return this;
  };
  __Timeout.prototype[Symbol.toPrimitive] = function() {
    return this._id;
  };
  __Timeout.prototype.valueOf = function() {
    return this._id;
  };

  globalThis.setTimeout = function(cb, ms) {
    var a = undefined;
    if (arguments.length > 2) {
      a = [];
      for (var i = 2; i < arguments.length; i++) a.push(arguments[i]);
    }
    return new __Timeout(__slice_timer_set(cb, Number(ms) || 0, false, a));
  };
  globalThis.clearTimeout = function(id) {
    if (id == null) return;
    __slice_timer_ctl(typeof id === "object" ? (id._id | 0) : (Number(id) || 0), 0);
  };
  globalThis.setInterval = function(cb, ms) {
    var a = undefined;
    if (arguments.length > 2) {
      a = [];
      for (var i = 2; i < arguments.length; i++) a.push(arguments[i]);
    }
    return new __Timeout(__slice_timer_set(cb, Number(ms) || 0, true, a));
  };
  globalThis.clearInterval = globalThis.clearTimeout;
  globalThis.setImmediate = function(cb) {
    if (typeof cb !== "function") {
      throw new TypeError("setImmediate requires a callback function");
    }
    var a = undefined;
    if (arguments.length > 1) {
      a = [];
      for (var i = 1; i < arguments.length; i++) a.push(arguments[i]);
    }
    return new __Timeout(__slice_immediate_set(cb, a));
  };
  globalThis.clearImmediate = globalThis.clearTimeout;

  globalThis.__slice_toU8 = function(input) {
    if (input === undefined || input === null) return new Uint8Array(0);
    if (input instanceof Uint8Array) return input;
    if (input instanceof ArrayBuffer) return new Uint8Array(input);
    if (input.buffer instanceof ArrayBuffer) {
      return new Uint8Array(input.buffer, input.byteOffset || 0, input.byteLength);
    }
    if (input._isBlob && input._u8 instanceof Uint8Array) return input._u8;
    if (typeof input === "string") return __slice_text_encode(input);
    if (Array.isArray(input)) return new Uint8Array(input);
    return __slice_text_encode(String(input));
  };

  globalThis.__slice_concatU8 = function(chunks) {
    if (chunks.length === 0) return new Uint8Array(0);
    if (chunks.length === 1) return new Uint8Array(chunks[0]);
    var total = 0;
    for (var i = 0; i < chunks.length; i++) total += chunks[i].byteLength;
    var out = new Uint8Array(total);
    var off = 0;
    for (var j = 0; j < chunks.length; j++) {
      out.set(chunks[j], off);
      off += chunks[j].byteLength;
    }
    return out;
  };

  function __tMod() {
    return __slice_builtins["slice:test"] || __slice_load_builtin("slice:test");
  }
  globalThis.describe = function(n, f) { return __tMod().describe(n, f); };
  globalThis.test = globalThis.it = function(n, f, o) { return __tMod().test(n, f, o); };
  globalThis.beforeEach = function(f) { return __tMod().beforeEach(f); };
  globalThis.afterEach = function(f) { return __tMod().afterEach(f); };
  globalThis.expect = function(v) { return __tMod().expect(v); };

  function __drainWu() {
    if (__wu.length === 0) return;
    try {
      while (__wu.length > 0) {
        var f = __wu.shift();
        if (typeof f === "function") f();
      }
    } finally {
      __wu.length = 0;
    }
  }
  globalThis.Slice = {
    request: function() {
      var t = __slice_http_tenant();
      return {
        method: __slice_http_method(),
        path: __slice_http_path(),
        query: __slice_http_query(),
        tenant: t,
        tenantId: t,
        body: __slice_http_body(),
        header: __slice_http_header
      };
    },
    respond: function(s, b, ct) {
      if (
        b !== null &&
        typeof b === "object" &&
        (b instanceof Uint8Array || b instanceof ArrayBuffer || (b.buffer instanceof ArrayBuffer))
      ) {
        return __slice_http_respond(s, __slice_toU8(b), ct || "application/octet-stream", "");
      }
      return __slice_http_respond(
        s,
        typeof b === "string" ? b : JSON.stringify(b),
        ct || "application/json",
        ""
      );
    },
    streamSSE: function(src, init) {
      __loadShard(10);
      return globalThis.Slice.streamSSE(src, init);
    },
    fetchSync: function(input, init) {
      __loadShard(10);
      return globalThis.Slice.fetchSync(input, init);
    },
    mockFetch: function(a, b, c) {
      __loadShard(10);
      return globalThis.Slice.mockFetch(a, b, c);
    },
    mockWebSocket: function(urlPattern, handlerOrSpec) {
      __loadShard(7);
      return globalThis.Slice.mockWebSocket(urlPattern, handlerOrSpec);
    },
    onWebSocket: function(handler) {
      __loadShard(7);
      return globalThis.Slice.onWebSocket(handler);
    },
    upgradeWebSocket: function(handlerOrOpts) {
      __loadShard(7);
      return globalThis.Slice.upgradeWebSocket(handlerOrOpts);
    },
    connect: function(addr, opts) {
      __loadShard(8);
      return globalThis.Slice.connect(addr, opts);
    },
    mockSocket: function(addr, spec) {
      __loadShard(8);
      return globalThis.Slice.mockSocket(addr, spec);
    },
    waitUntil: function(f) {
      __wu.push(f);
      __slice_mark_wait_until();
    },
    kv: {
      get: function(k) { return __slice_kv_get(String(k)); },
      set: function(k, v, ttl) {
        return __slice_kv_set(String(k), typeof v === "string" ? v : JSON.stringify(v), ttl || 0);
      },
      del: function(k) { return __slice_kv_del(String(k)); },
      incr: function(k, d) { return __slice_kv_incr(String(k), d === undefined ? 1 : Number(d)); }
    },
    checkpoint: function() { return __slice_checkpoint(); },
    restore: function(id) { return __slice_restore(Number(id)); },
    releaseCheckpoint: function(id) { return __slice_release_checkpoint(Number(id)); },
    evalSandbox: function(code, opts) {
      return JSON.parse(__slice_eval_sandbox(String(code), opts ? JSON.stringify(opts) : "{}"));
    },
    serve: function(optsOrHandler, maybeHandler) {
      __loadShard(1);
      __loadShard(11);
      var handler = typeof optsOrHandler === "function" ? optsOrHandler : (typeof maybeHandler === "function" ? maybeHandler : (optsOrHandler && typeof optsOrHandler.fetch === "function" ? optsOrHandler : null));
      var o = (optsOrHandler && typeof optsOrHandler === "object") ? optsOrHandler : {};
      var port = Number(o.port) || 8787;
      var hostname = String(o.hostname || o.host || "127.0.0.1");
      if (handler) {
        globalThis.__slice_active_http_server = {
          _handler: handler,
          _dispatchActiveHttpBridge: function() {
            globalThis.__slice_dispatch_fetch(handler);
          }
        };
        if (typeof __slice_http_is_active === "function" && __slice_http_is_active()) {
          globalThis.__slice_dispatch_fetch(handler);
        }
      }
      if (typeof o.onListen === "function") {
        queueMicrotask(function() { o.onListen({ port: port, hostname: hostname }); });
      }
      return {
        port: port,
        hostname: hostname,
        addr: { transport: "tcp", hostname: hostname, port: port },
        finished: Promise.resolve(),
        stop: function() { globalThis.__slice_active_http_server = null; },
        shutdown: function() { globalThis.__slice_active_http_server = null; return Promise.resolve(); },
        ref: function() {},
        unref: function() {}
      };
    },
    testSummary: function() { return __tMod().summary(); },
    resetTests: function() { return __tMod().reset(); }
  };
  globalThis.Deno = globalThis.Deno || { serve: globalThis.Slice.serve };
  globalThis.Bun = globalThis.Bun || { serve: globalThis.Slice.serve };

  globalThis.__slice_builtins = Object.create(null);
  globalThis.__slice_cjs_require = function(id) {
    return __slice_builtins[id] || __slice_load_builtin(id);
  };
  globalThis.require = __slice_cjs_require;
  globalThis.escape = function(str) {
    var s = String(str), out = "";
    for (var i = 0; i < s.length; i++) {
      var c = s.charCodeAt(i);
      if ((c >= 48 && c <= 57) || (c >= 64 && c <= 90) || (c >= 97 && c <= 122) || c === 42 || c === 43 || c === 45 || c === 46 || c === 47 || c === 95) {
        out += s.charAt(i);
      } else if (c < 256) {
        var h = c.toString(16).toUpperCase();
        out += "%" + (h.length < 2 ? "0" + h : h);
      } else {
        var u = c.toString(16).toUpperCase();
        while (u.length < 4) u = "0" + u;
        out += "%u" + u;
      }
    }
    return out;
  };
  globalThis.unescape = function(str) {
    var s = String(str);
    if (s.indexOf("%") === -1) return s;
    return s.replace(/%u([0-9a-fA-F]{4})|%([0-9a-fA-F]{2})/g, function(_, u4, x2) {
      return String.fromCharCode(parseInt(u4 || x2, 16));
    });
  };
  var __errStackDesc = Object.getOwnPropertyDescriptor(Error.prototype, "stack");
  if (__errStackDesc && __errStackDesc.get) {
    var __origStackGet = __errStackDesc.get;
    Object.defineProperty(Error.prototype, "stack", {
      get: function() {
        var raw = __origStackGet.call(this);
        var str = raw ? String(raw) : "";
        var mapped = typeof __slice_map_error_stack === "function" ? __slice_map_error_stack(str) : str;
        var name = (this && this.name) ? String(this.name) : "Error";
        var msg = (this && this.message) ? String(this.message) : "";
        var hdr = msg ? (name + ": " + msg) : name;
        if (mapped.indexOf(hdr) === 0) return mapped;
        return mapped ? (hdr + "\n" + mapped) : hdr;
      },
      set: __errStackDesc.set,
      configurable: true,
      enumerable: false
    });
  }
  var __origArrFill = Array.prototype.fill;
  Object.defineProperty(Array.prototype, "fill", {
    value: function(v, s, e) {
      var len = this ? (this.length >>> 0) : 0;
      if (len > 64) {
        var st = s === undefined ? 0 : (s < 0 ? Math.max(len + Number(s), 0) : Math.min(Number(s), len));
        var en = e === undefined ? len : (e < 0 ? Math.max(len + Number(e), 0) : Math.min(Number(e), len));
        var cnt = en > st ? (en - st) : 0;
        if (cnt > 64) __slice_charge_mem(cnt * 8, Math.floor(cnt / 16) + 1);
      }
      return __origArrFill.apply(this, arguments);
    },
    writable: true,
    configurable: true,
    enumerable: false
  });
  var __origStrRepeat = String.prototype.repeat;
  Object.defineProperty(String.prototype, "repeat", {
    value: function(count) {
      var n = Number(count) || 0;
      var bytes = String(this).length * (n > 0 ? Math.floor(n) : 0);
      if (bytes > 64) __slice_charge_mem(bytes, Math.floor(bytes / 64) + 1);
      return __origStrRepeat.call(this, count);
    },
    writable: true,
    configurable: true,
    enumerable: false
  });
  var __origObjToStr = Object.prototype.toString;
  var __origObjValOf = Object.prototype.valueOf;
  var __origObjHasOwn = Object.prototype.hasOwnProperty;
  __sesP = [
    Object.prototype,
    Array.prototype,
    Function.prototype,
    String.prototype,
    Promise.prototype,
    Uint8Array.prototype
  ];
  __sesN = [
    "Object.prototype",
    "Array.prototype",
    "Function.prototype",
    "String.prototype",
    "Promise.prototype",
    "Uint8Array.prototype"
  ];
  __sesK = [
    Reflect.ownKeys(Object.prototype),
    Reflect.ownKeys(Array.prototype),
    Reflect.ownKeys(Function.prototype),
    Reflect.ownKeys(String.prototype),
    Reflect.ownKeys(Promise.prototype),
    Reflect.ownKeys(Uint8Array.prototype)
  ];
  __sesL = [
    __sesK[0].length,
    __sesK[1].length,
    __sesK[2].length,
    __sesK[3].length,
    __sesK[4].length,
    __sesK[5].length
  ];
  __sesReady = true;
  function __checkSes() {
    if (
      Object.prototype.toString !== __origObjToStr ||
      Object.prototype.valueOf !== __origObjValOf ||
      Object.prototype.hasOwnProperty !== __origObjHasOwn
    ) {
      return "Object.prototype";
    }
    if (!__ntHooked && Promise.prototype.then !== __origThen) {
      return "Promise.prototype";
    }
    for (var i = 0; i < 6; i++) {
      var p = __sesP[i];
      var ks = Reflect.ownKeys(p);
      var base = __sesK[i];
      var mutated = ks.length !== __sesL[i];
      if (!mutated) {
        for (var k = 0; k < ks.length; k++) {
          if (ks[k] !== base[k]) {
            mutated = true;
            break;
          }
        }
      }
      if (mutated) {
        for (var j = 0; j < ks.length; j++) {
          if (base.indexOf(ks[j]) === -1) {
            try { delete p[ks[j]]; } catch (_e) {}
          }
        }
        return __sesN[i];
      }
    }
    return "";
  }
  Object.defineProperties(globalThis, {
    __slice_check_ses: { value: __checkSes, writable: false, enumerable: false, configurable: false },
    __slice_sync_process_env: { value: __syncProcessEnv, writable: false, enumerable: false, configurable: false },
    __slice_set_det_mode: { value: __setDetMode, writable: false, enumerable: false, configurable: false },
    __slice_set_next_tick_hook: { value: __setNextTickHook, writable: false, enumerable: false, configurable: false },
    __slice_reset_mt: { value: __resetMt, writable: false, enumerable: false, configurable: false },
    __slice_ctl: { value: __ctl, writable: false, enumerable: false, configurable: false },
    __slice_drain_wu: { value: __drainWu, writable: false, enumerable: false, configurable: false }
  });
})();

