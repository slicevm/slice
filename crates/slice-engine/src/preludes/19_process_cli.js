// Slice (Nexus-JS) Prelude Shard 29: `navigator`, `location`, and Complete `process` CLI Surface (`19_process_cli.js`)
(function() {
  var p = globalThis.process;
  if (p && !p.stdout) {
    p.ppid = 0;
    p.argv0 = "slice";
    p.execArgv = [];
    p.execPath = "/usr/bin/slice";
    var __t0 = Date.now();
    p.uptime = function() {
      return Math.max(0, (Date.now() - __t0) / 1000);
    };
    function __hrtime(prev) {
      var ns = BigInt(Math.round(performance.now() * 1e6));
      if (Array.isArray(prev) && prev.length === 2) {
        var pNs = BigInt(prev[0]) * 1000000000n + BigInt(prev[1]);
        ns -= pNs;
        if (ns < 0n) ns = 0n;
      }
      return [Number(ns / 1000000000n), Number(ns % 1000000000n)];
    }
    __hrtime.bigint = function() {
      return BigInt(Math.round(performance.now() * 1e6));
    };
    p.hrtime = __hrtime;

    function __memUsage() {
      var used = __slice_http_is_active(1) || 0;
      var rem = __slice_http_is_active(2) || 67108864;
      var total = Math.min(used + rem, 134217728);
      var heapUsed = Math.max(262144, used + 262144);
      var heapTotal = Math.max(heapUsed, Math.min(total, 16777216));
      var rss = Math.max(heapTotal, 8388608);
      return {
        rss: rss,
        heapTotal: heapTotal,
        heapUsed: heapUsed,
        external: used,
        arrayBuffers: 0
      };
    }
    __memUsage.rss = function() {
      return __memUsage().rss;
    };
    p.memoryUsage = __memUsage;

    function __mkWritable(fd) {
      return {
        fd: fd,
        isTTY: false,
        columns: 80,
        rows: 24,
        write: function(chunk, enc, cb) {
          var fn = typeof enc === "function" ? enc : cb;
          var s = (chunk instanceof Uint8Array)
            ? __slice_buf_to_utf8(chunk)
            : String(chunk === undefined ? "" : chunk);
          if (s.endsWith("\r\n")) s = s.slice(0, -2);
          else if (s.endsWith("\n")) s = s.slice(0, -1);
          __slice_console_log(s);
          if (typeof fn === "function") fn(null);
          return true;
        },
        end: function(chunk, enc, cb) {
          var fn = typeof chunk === "function" ? chunk : (typeof enc === "function" ? enc : cb);
          if (chunk !== undefined && chunk !== null && typeof chunk !== "function") {
            this.write(chunk, enc);
          }
          if (typeof fn === "function") fn();
          return this;
        },
        on: function() { return this; },
        once: function() { return this; },
        off: function() { return this; },
        removeListener: function() { return this; },
        emit: function() { return false; }
      };
    }
    p.stdout = __mkWritable(1);
    p.stderr = __mkWritable(2);
    p.stdin = {
      fd: 0,
      isTTY: false,
      readable: false,
      on: function(ev, cb) {
        if (ev === "end" && typeof cb === "function") queueMicrotask(cb);
        return this;
      },
      once: function(ev, cb) {
        if (ev === "end" && typeof cb === "function") queueMicrotask(cb);
        return this;
      },
      off: function() { return this; },
      removeListener: function() { return this; },
      resume: function() { return this; },
      pause: function() { return this; },
      setEncoding: function() { return this; },
      read: function() { return null; }
    };

    var __pEv = Object.create(null);
    var __pEvCount = 0;
    p._exited = false;
    p.on = p.addListener = function(ev, fn) {
      if (typeof fn === "function") {
        __slice_process_ctl(0);
        var k = String(ev);
        (__pEv[k] || (__pEv[k] = [])).push(fn);
        __pEvCount++;
      }
      return p;
    };
    p.once = function(ev, fn) {
      if (typeof fn === "function") {
        var g = function() {
          p.off(ev, g);
          fn.apply(p, arguments);
        };
        g.listener = fn;
        p.on(ev, g);
      }
      return p;
    };
    p.off = p.removeListener = function(ev, fn) {
      var k = String(ev), list = __pEv[k];
      if (list) {
        var prevLen = list.length;
        __pEv[k] = list.filter(function(f) { return f !== fn && f.listener !== fn; });
        __pEvCount = Math.max(0, __pEvCount - (prevLen - __pEv[k].length));
      }
      return p;
    };
    p.removeAllListeners = function(ev) {
      if (arguments.length === 0) {
        __pEv = Object.create(null);
        __pEvCount = 0;
      } else {
        var k = String(ev);
        if (__pEv[k]) {
          __pEvCount = Math.max(0, __pEvCount - __pEv[k].length);
          delete __pEv[k];
        }
      }
      return p;
    };
    p.listeners = function(ev) {
      return (__pEv[String(ev)] || []).slice();
    };
    p.listenerCount = function(ev) {
      var l = __pEv[String(ev)];
      return l ? l.length : 0;
    };
    p._hasListeners = function(ev) {
      if (__pEvCount === 0) return false;
      if (!ev) return true;
      var l = __pEv[String(ev)];
      return Boolean(l && l.length > 0);
    };
    p._resetProcessState = function() {
      if (__pEvCount > 0) {
        __pEv = Object.create(null);
        __pEvCount = 0;
      }
      if (p._resetExitCode) p._resetExitCode();
      else p.exitCode = undefined;
      p._exited = false;
    };
    p.emit = function(ev) {
      if (__pEvCount === 0) return false;
      var list = __pEv[String(ev)];
      if (!list || list.length === 0) return false;
      var args = [];
      for (var i = 1; i < arguments.length; i++) args.push(arguments[i]);
      var snap = list.slice();
      for (var j = 0; j < snap.length; j++) snap[j].apply(p, args);
      return true;
    };
    p.emitWarning = function(warning, type, code) {
      var msg = (warning && typeof warning === "object" && warning.message)
        ? warning.message
        : String(warning);
      var t = typeof type === "string"
        ? type
        : (type && typeof type === "object" && type.type ? type.type : "Warning");
      var c = typeof code === "string"
        ? code
        : (type && typeof type === "object" && type.code ? type.code : undefined);
      var w = new Error(msg);
      w.name = t;
      if (c) w.code = c;
      p.emit("warning", w);
    };
  }

  if (!globalThis.navigator) {
    globalThis.navigator = {
      userAgent: "Slice-NexusJS/0.10.0",
      hardwareConcurrency: 4,
      language: "en-US",
      languages: ["en-US", "en"],
      onLine: true,
      platform: "Slice",
      maxTouchPoints: 0
    };
  }

  if (!globalThis.location) {
    globalThis.location = {
      href: "http://localhost/",
      origin: "http://localhost",
      protocol: "http:",
      host: "localhost",
      hostname: "localhost",
      port: "",
      pathname: "/",
      search: "",
      hash: "",
      toString: function() { return this.href; }
    };
  }
})();
