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
        (id === 18 && globalThis.Intl)
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
    }
  }
  globalThis.__slice_ensure_shard = __loadShard;

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

  globalThis.process = {
    pid: 1,
    env: { NODE_ENV: "production" },
    cwd: function() { return "."; },
    get platform() { return __slice_os_platform(); },
    get arch() { return __slice_os_arch(); },
    version: "v24.0.0",
    versions: { node: "24.0.0" },
    argv: ["slice"]
  };

  globalThis.__slice_check_ses = function() {
    var p = [Object.prototype, Array.prototype, Function.prototype, String.prototype];
    var n = ["Object.prototype", "Array.prototype", "Function.prototype", "String.prototype"];
    for (var i = 0; i < 4; i++) {
      var ks = Object.keys(p[i]);
      if (ks.length > 0) {
        for (var j = 0; j < ks.length; j++) delete p[i][ks[j]];
        return n[i];
      }
    }
    return "";
  };

  globalThis.__slice_sync_process_env = function(load) {
    if (!load) {
      globalThis.process.env = { NODE_ENV: "production" };
      return;
    }
    var m = JSON.parse(__slice_env_json());
    m.NODE_ENV = m.NODE_ENV || "production";
    globalThis.process.env = m;
  };

  var __orand = Math.random;
  var __onow = Date.now;
  function __perfNowNative() {
    return __onow() - 1700000000000;
  }
  globalThis.performance = {
    now: __perfNowNative
  };
  globalThis.__slice_set_det_mode = function(on) {
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
  };

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
    var fF = (__alsWrap && typeof onF === "function") ? __alsWrap(onF) : onF;
    var fR = (__alsWrap && typeof onR === "function") ? __alsWrap(onR) : onR;
    var wF = typeof fF === "function" ? function(v) {
      if (__ntQ.length > 0) {
        while (__ntQ.length > 0) {
          var n = __ntQ.shift();
          n.f.apply(undefined, n.a);
        }
      }
      return fF(v);
    } : fF;
    var wR = typeof fR === "function" ? function(e) {
      if (__ntQ.length > 0) {
        while (__ntQ.length > 0) {
          var n = __ntQ.shift();
          n.f.apply(undefined, n.a);
        }
      }
      return fR(e);
    } : fR;
    return __origThen.call(this, wF, wR);
  }

  globalThis.__slice_set_next_tick_hook = function(enable) {
    if (enable && !__ntHooked) {
      __ntHooked = true;
      Promise.prototype.then = __wrappedThen;
    } else if (!enable && __ntHooked && __ntQ.length === 0 && !__alsWrap) {
      __ntHooked = false;
      Promise.prototype.then = __origThen;
    }
  };

  globalThis.__slice_set_als_wrap = function(wrapFn) {
    __alsWrap = wrapFn || null;
    if (__alsWrap && !__ntHooked) {
      __ntHooked = true;
      Promise.prototype.then = __wrappedThen;
    }
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
        n.f.apply(undefined, n.a);
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
    var a = [];
    for (var i = 1; i < arguments.length; i++) a.push(arguments[i]);
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

  globalThis.setTimeout = function(cb, ms) {
    var a = [];
    for (var i = 2; i < arguments.length; i++) a.push(arguments[i]);
    return __slice_timer_set(cb, Number(ms) || 0, false, a);
  };
  globalThis.clearTimeout = function(id) {
    __slice_timer_clear(Number(id) || 0);
  };
  globalThis.setInterval = function(cb, ms) {
    var a = [];
    for (var i = 2; i < arguments.length; i++) a.push(arguments[i]);
    return __slice_timer_set(cb, Number(ms) || 0, true, a);
  };
  globalThis.clearInterval = globalThis.clearTimeout;
  globalThis.setImmediate = function(cb) {
    if (typeof cb !== "function") {
      throw new TypeError("setImmediate requires a callback function");
    }
    var a = [];
    for (var i = 1; i < arguments.length; i++) a.push(arguments[i]);
    return __slice_immediate_set(cb, a);
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

  globalThis.__slice_wu = [];
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
      __slice_wu.push(f);
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
})();

