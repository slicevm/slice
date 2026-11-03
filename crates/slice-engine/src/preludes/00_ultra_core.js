// Slice (Nexus-JS) Ultra-Core Bootstrap Prelude (`00_ultra_core.js`)
// Evaluated eagerly on realm creation; registers self-replacing lazy getters (`__defLazy`)
// for Shards 1..=7 so unused WinterTC/Node.js subsystems incur 0 bytes of parse/eval cost.
(function() {
  var __shMask = 0;
  var __shardProps = {};
  function __loadShard(id) {
    if ((id === 1 || id === 3 || id === 7) && (__shMask & 4) === 0) {
      __loadShard(2);
    }
    var bit = 1 << id;
    if ((__shMask & bit) === 0) {
      __shMask |= bit;
      var ns = __shardProps[id];
      if (ns) {
        for (var i = 0; i < ns.length; i++) delete globalThis[ns[i]];
      }
      __slice_lazy_prelude(id);
    }
  }
  globalThis.__slice_ensure_shard = __loadShard;

  function __defLazy(names, shardId) {
    __shardProps[shardId] = names;
    for (var i = 0; i < names.length; i++) {
      (function(prop) {
        Object.defineProperty(globalThis, prop, {
          configurable: true,
          enumerable: false,
          get: function() {
            __loadShard(shardId);
            return globalThis[prop];
          },
          set: function(val) {
            delete globalThis[prop];
            globalThis[prop] = val;
          }
        });
      })(names[i]);
    }
  }

  var __lazyInited = false;
  globalThis.__slice_init_lazy_getters = function() {
    if (__lazyInited) return;
    __lazyInited = true;
    if ((__shMask & 2) === 0) {
      __defLazy([
        "URLSearchParams",
        "URL",
        "Headers",
        "Request",
        "Response",
        "AbortSignal",
        "AbortController",
        "fetch",
        "__slice_make_web_request"
      ], 1);
    }
    if ((__shMask & 4) === 0) {
      __defLazy([
        "btoa",
        "atob",
        "TextEncoder",
        "TextDecoder",
        "Event",
        "CustomEvent",
        "EventTarget"
      ], 2);
    }
    if ((__shMask & 8) === 0) {
      __defLazy([
        "ReadableStream",
        "WritableStream",
        "TransformStream",
        "TextEncoderStream",
        "TextDecoderStream",
        "Blob",
        "File",
        "FormData"
      ], 3);
    }
    if ((__shMask & 16) === 0) {
      __defLazy([
        "crypto",
        "CryptoKey",
        "structuredClone"
      ], 4);
    }
    if ((__shMask & 32) === 0) {
      __defLazy(["Buffer"], 5);
    }
    if ((__shMask & 64) === 0) {
      var __cMethods = [
        "dir", "dirxml", "table", "trace", "assert", "clear",
        "count", "countReset", "group", "groupCollapsed", "groupEnd",
        "time", "timeLog", "timeEnd"
      ];
      for (var c = 0; c < __cMethods.length; c++) {
        (function(m) {
          globalThis.console[m] = function() {
            __loadShard(6);
            return globalThis.console[m].apply(globalThis.console, arguments);
          };
        })(__cMethods[c]);
      }
    }
    if ((__shMask & 128) === 0) {
      __defLazy([
        "WebSocket",
        "MessageEvent",
        "CloseEvent",
        "__slice_dispatch_server_ws"
      ], 7);
    }
  };

  globalThis.__slice_dispatch_fetch = function(h) {
    if (!__slice_http_is_active()) return;
    __loadShard(1);
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
  Math.random = function() {
    var v = __slice_det_random();
    return v >= 0 ? v : __orand();
  };
  Date.now = function() {
    var v = __slice_det_now();
    return v >= 0 ? v : __onow();
  };
  globalThis.performance = {
    now: function() {
      var v = __slice_det_now();
      return v >= 0 ? (v - 1700000000000) : (__onow() - 1700000000000);
    }
  };

  // `process.nextTick` & `queueMicrotask`:
  // Fast check (`if (__ntQ.length > 0)`) in `Promise.prototype.then` avoids loop overhead
  // when no `nextTick` callbacks are queued while ensuring `nextTick` drains before `.then()`.
  var __ntQ = [];
  var __qmQ = [];
  var __mtS = false;
  var __origThen = Promise.prototype.then;

  Promise.prototype.then = function(onF, onR) {
    var wF = typeof onF === "function" ? function(v) {
      if (__ntQ.length > 0) {
        while (__ntQ.length > 0) {
          var n = __ntQ.shift();
          n.f.apply(undefined, n.a);
        }
      }
      return onF(v);
    } : onF;
    var wR = typeof onR === "function" ? function(e) {
      if (__ntQ.length > 0) {
        while (__ntQ.length > 0) {
          var n = __ntQ.shift();
          n.f.apply(undefined, n.a);
        }
      }
      return onR(e);
    } : onR;
    return __origThen.call(this, wF, wR);
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
    var a = [];
    for (var i = 1; i < arguments.length; i++) a.push(arguments[i]);
    __ntQ.push({ f: cb, a: a });
    __schedMt();
  };

  globalThis.queueMicrotask = function(cb) {
    if (typeof cb !== "function") {
      throw new TypeError("queueMicrotask requires a callback function");
    }
    __qmQ.push(cb);
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
      __loadShard(1);
      return globalThis.Slice.streamSSE(src, init);
    },
    fetchSync: function(input, init) {
      __loadShard(1);
      return globalThis.Slice.fetchSync(input, init);
    },
    mockFetch: function(a, b, c) {
      __loadShard(1);
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
    testSummary: function() { return __tMod().summary(); },
    resetTests: function() { return __tMod().reset(); }
  };

  globalThis.__slice_builtins = Object.create(null);
  globalThis.__slice_cjs_require = function(id) {
    return __slice_builtins[id] || __slice_load_builtin(id);
  };
  globalThis.require = __slice_cjs_require;
})();

