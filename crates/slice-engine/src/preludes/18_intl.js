// Slice (Nexus-JS) Prelude Shard 18: WinterTC `navigator`, `location`, `Intl`, and Complete `process` CLI Surface
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
        __slice_mark_proc_dirty();
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

  function __normLocale(loc) {
    if (!loc) return "en-US";
    var first = Array.isArray(loc) ? loc[0] : loc;
    if (!first || typeof first !== "string") return "en-US";
    var parts = first.trim().replace(/_/g, "-").split("-");
    if (parts.length === 1) return parts[0].toLowerCase();
    return parts[0].toLowerCase() + "-" + parts[1].toUpperCase() + (parts.length > 2 ? "-" + parts.slice(2).join("-") : "");
  }

  function __pad2(n) {
    return n < 10 ? "0" + n : String(n);
  }

  var __MONTHS_SHORT = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
  var __MONTHS_LONG = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
  var __DAYS_SHORT = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
  var __DAYS_LONG = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

  function DateTimeFormat(locales, options) {
    if (!(this instanceof DateTimeFormat)) return new DateTimeFormat(locales, options);
    var o = options || {};
    this._locale = __normLocale(locales);
    this._timeZone = o.timeZone || "UTC";
    this._dateStyle = o.dateStyle;
    this._timeStyle = o.timeStyle;
    var hasCustom = o.year || o.month || o.day || o.weekday || o.hour || o.minute || o.second || o.dateStyle || o.timeStyle;
    this._year = o.year || (!hasCustom || o.dateStyle ? "numeric" : undefined);
    this._month = o.month || (!hasCustom ? "numeric" : (o.dateStyle === "full" || o.dateStyle === "long" ? "long" : (o.dateStyle === "medium" ? "short" : (o.dateStyle === "short" ? "numeric" : undefined))));
    this._day = o.day || (!hasCustom || o.dateStyle ? "numeric" : undefined);
    this._weekday = o.weekday || (o.dateStyle === "full" ? "long" : undefined);
    this._hour = o.hour || (o.timeStyle ? "numeric" : undefined);
    this._minute = o.minute || (o.timeStyle ? "2-digit" : undefined);
    this._second = o.second || (o.timeStyle === "full" || o.timeStyle === "long" || o.timeStyle === "medium" ? "2-digit" : undefined);
    this._hour12 = o.hour12 !== undefined ? Boolean(o.hour12) : (o.hourCycle ? (o.hourCycle === "h11" || o.hourCycle === "h12") : false);
    this._timeZoneName = o.timeZoneName || (o.timeStyle === "full" || o.timeStyle === "long" ? "short" : undefined);
  }
  DateTimeFormat.prototype.resolvedOptions = function() {
    return {
      locale: this._locale,
      calendar: "gregory",
      numberingSystem: "latn",
      timeZone: this._timeZone,
      hour12: this._hour12,
      year: this._year,
      month: this._month,
      day: this._day,
      weekday: this._weekday,
      hour: this._hour,
      minute: this._minute,
      second: this._second
    };
  };
  DateTimeFormat.prototype.formatToParts = function(date) {
    var d = date === undefined ? new Date(Date.now()) : (date instanceof Date ? date : new Date(Number(date)));
    if (Number.isNaN(d.getTime())) throw new RangeError("Invalid time value");
    var parts = [];
    if (this._weekday) {
      var wd = d.getUTCDay();
      var wVal = this._weekday === "long" ? __DAYS_LONG[wd] : (this._weekday === "narrow" ? __DAYS_SHORT[wd].charAt(0) : __DAYS_SHORT[wd]);
      parts.push({ type: "weekday", value: wVal });
      if (this._month || this._day || this._year) parts.push({ type: "literal", value: ", " });
    }
    var hasDate = Boolean(this._year || this._month || this._day);
    if (hasDate) {
      var mIdx = d.getUTCMonth();
      var dayNum = d.getUTCDate();
      var yrNum = d.getUTCFullYear();
      var yrStr = this._year === "2-digit" ? __pad2(yrNum % 100) : String(yrNum);
      var dayStr = this._day === "2-digit" ? __pad2(dayNum) : String(dayNum);
      if (this._month === "long" || this._month === "short" || this._month === "narrow") {
        var mStr = this._month === "long" ? __MONTHS_LONG[mIdx] : (this._month === "narrow" ? __MONTHS_SHORT[mIdx].charAt(0) : __MONTHS_SHORT[mIdx]);
        parts.push({ type: "month", value: mStr });
        if (this._day) {
          parts.push({ type: "literal", value: " " });
          parts.push({ type: "day", value: dayStr });
        }
        if (this._year) {
          parts.push({ type: "literal", value: this._day ? ", " : " " });
          parts.push({ type: "year", value: yrStr });
        }
      } else {
        var mNumStr = this._month === "2-digit" ? __pad2(mIdx + 1) : String(mIdx + 1);
        var isoOrder = this._locale.indexOf("sv") === 0 || this._locale.indexOf("ja") === 0 || this._locale.indexOf("zh") === 0 || this._locale.indexOf("iso") === 0;
        if (isoOrder) {
          if (this._year) parts.push({ type: "year", value: yrStr });
          if (this._year && this._month) parts.push({ type: "literal", value: "-" });
          if (this._month) parts.push({ type: "month", value: __pad2(mIdx + 1) });
          if (this._month && this._day) parts.push({ type: "literal", value: "-" });
          if (this._day) parts.push({ type: "day", value: __pad2(dayNum) });
        } else {
          if (this._month) parts.push({ type: "month", value: mNumStr });
          if (this._month && this._day) parts.push({ type: "literal", value: "/" });
          if (this._day) parts.push({ type: "day", value: dayStr });
          if ((this._month || this._day) && this._year) parts.push({ type: "literal", value: "/" });
          if (this._year) parts.push({ type: "year", value: yrStr });
        }
      }
    }
    var hasTime = Boolean(this._hour || this._minute || this._second);
    if (hasDate && hasTime) {
      parts.push({ type: "literal", value: ", " });
    }
    if (hasTime) {
      var hr = d.getUTCHours();
      var ampm = hr >= 12 ? "PM" : "AM";
      var dispHr = this._hour12 ? (hr % 12 || 12) : hr;
      if (this._hour) {
        parts.push({ type: "hour", value: this._hour === "2-digit" ? __pad2(dispHr) : String(dispHr) });
      }
      if (this._minute) {
        if (this._hour) parts.push({ type: "literal", value: ":" });
        parts.push({ type: "minute", value: __pad2(d.getUTCMinutes()) });
      }
      if (this._second) {
        if (this._hour || this._minute) parts.push({ type: "literal", value: ":" });
        parts.push({ type: "second", value: __pad2(d.getUTCSeconds()) });
      }
      if (this._hour12) {
        parts.push({ type: "literal", value: " " });
        parts.push({ type: "dayPeriod", value: ampm });
      }
      if (this._timeZoneName) {
        parts.push({ type: "literal", value: " " });
        parts.push({ type: "timeZoneName", value: this._timeZone });
      }
    }
    return parts;
  };
  DateTimeFormat.prototype.format = function(date) {
    var parts = this.formatToParts(date);
    var out = "";
    for (var i = 0; i < parts.length; i++) out += parts[i].value;
    return out;
  };
  DateTimeFormat.supportedLocalesOf = function(loc) {
    return loc ? [__normLocale(loc)] : ["en-US"];
  };

  var __CURR_SYM = { USD: "$", EUR: "€", GBP: "£", JPY: "¥", ILS: "₪", CAD: "CA$", AUD: "A$" };

  function NumberFormat(locales, options) {
    if (!(this instanceof NumberFormat)) return new NumberFormat(locales, options);
    var o = options || {};
    this._locale = __normLocale(locales);
    this._style = o.style || "decimal";
    this._currency = o.currency ? String(o.currency).toUpperCase() : undefined;
    this._currencyDisplay = o.currencyDisplay || "symbol";
    this._unit = o.unit;
    this._unitDisplay = o.unitDisplay || "short";
    this._notation = o.notation || "standard";
    this._signDisplay = o.signDisplay || "auto";
    this._useGrouping = o.useGrouping !== undefined ? Boolean(o.useGrouping) : true;
    this._minInt = o.minimumIntegerDigits !== undefined ? Number(o.minimumIntegerDigits) : 1;
    var defMinFrac = this._style === "currency" ? (this._currency === "JPY" ? 0 : 2) : 0;
    var defMaxFrac = this._style === "currency" ? defMinFrac : (this._style === "percent" ? 0 : 3);
    this._minFrac = o.minimumFractionDigits !== undefined ? Number(o.minimumFractionDigits) : defMinFrac;
    this._maxFrac = o.maximumFractionDigits !== undefined ? Number(o.maximumFractionDigits) : Math.max(this._minFrac, defMaxFrac);
  }
  NumberFormat.prototype.resolvedOptions = function() {
    return {
      locale: this._locale,
      numberingSystem: "latn",
      style: this._style,
      currency: this._currency,
      currencyDisplay: this._currencyDisplay,
      unit: this._unit,
      unitDisplay: this._unitDisplay,
      notation: this._notation,
      signDisplay: this._signDisplay,
      useGrouping: this._useGrouping,
      minimumIntegerDigits: this._minInt,
      minimumFractionDigits: this._minFrac,
      maximumFractionDigits: this._maxFrac
    };
  };
  NumberFormat.prototype.formatToParts = function(value) {
    var num = Number(value);
    if (Number.isNaN(num)) return [{ type: "nan", value: "NaN" }];
    if (!Number.isFinite(num)) {
      var p = [];
      if (num < 0) p.push({ type: "minusSign", value: "-" });
      p.push({ type: "infinity", value: "∞" });
      return p;
    }
    var isNeg = num < 0 || Object.is(num, -0);
    var abs = Math.abs(num);
    if (this._style === "percent") abs *= 100;
    var compactSuffix = "";
    if (this._notation === "compact") {
      if (abs >= 1e12) { abs /= 1e12; compactSuffix = "T"; }
      else if (abs >= 1e9) { abs /= 1e9; compactSuffix = "B"; }
      else if (abs >= 1e6) { abs /= 1e6; compactSuffix = "M"; }
      else if (abs >= 1e3) { abs /= 1e3; compactSuffix = "K"; }
    }
    var fixed = abs.toFixed(compactSuffix ? Math.min(this._maxFrac, 1) : this._maxFrac);
    var split = fixed.split(".");
    var intStr = split[0];
    var fracStr = split[1] || "";
    if (!compactSuffix) {
      while (fracStr.length > this._minFrac && fracStr.charCodeAt(fracStr.length - 1) === 48) {
        fracStr = fracStr.slice(0, -1);
      }
    } else if (fracStr === "0") {
      fracStr = "";
    }
    while (intStr.length < this._minInt) intStr = "0" + intStr;

    var parts = [];
    if (this._signDisplay !== "never") {
      if (isNeg && (abs !== 0 || this._signDisplay !== "exceptZero")) {
        parts.push({ type: "minusSign", value: "-" });
      } else if (this._signDisplay === "always" || (this._signDisplay === "exceptZero" && abs > 0)) {
        parts.push({ type: "plusSign", value: "+" });
      }
    }
    if (this._style === "currency" && this._currency) {
      var sym = this._currencyDisplay === "code"
        ? (this._currency + " ")
        : (__CURR_SYM[this._currency] || (this._currency + " "));
      parts.push({ type: "currency", value: sym });
    }
    if (this._useGrouping && intStr.length > 3) {
      var groups = [];
      var rem = intStr;
      while (rem.length > 3) {
        groups.unshift(rem.slice(-3));
        rem = rem.slice(0, -3);
      }
      if (rem) groups.unshift(rem);
      for (var i = 0; i < groups.length; i++) {
        if (i > 0) parts.push({ type: "group", value: "," });
        parts.push({ type: "integer", value: groups[i] });
      }
    } else {
      parts.push({ type: "integer", value: intStr });
    }
    if (fracStr.length > 0) {
      parts.push({ type: "decimal", value: "." });
      parts.push({ type: "fraction", value: fracStr });
    }
    if (compactSuffix) {
      parts.push({ type: "compact", value: compactSuffix });
    }
    if (this._style === "percent") {
      parts.push({ type: "percentSign", value: "%" });
    } else if (this._style === "unit" && this._unit) {
      var uMap = { byte: "B", kilobyte: "kB", megabyte: "MB", millisecond: "ms", second: "s", minute: "min", hour: "hr", day: "d", percent: "%" };
      var uStr = this._unitDisplay === "long" ? (" " + this._unit + (abs === 1 ? "" : "s")) : (" " + (uMap[this._unit] || this._unit));
      parts.push({ type: "unit", value: uStr });
    }
    return parts;
  };
  NumberFormat.prototype.format = function(value) {
    var parts = this.formatToParts(value);
    var out = "";
    for (var i = 0; i < parts.length; i++) out += parts[i].value;
    return out;
  };
  NumberFormat.supportedLocalesOf = DateTimeFormat.supportedLocalesOf;

  function Collator(locales, options) {
    if (!(this instanceof Collator)) return new Collator(locales, options);
    var o = options || {};
    this._locale = __normLocale(locales);
    this._usage = o.usage || "sort";
    this._sensitivity = o.sensitivity || "variant";
    this._ignorePunctuation = Boolean(o.ignorePunctuation);
    this._numeric = Boolean(o.numeric);
    this._caseFirst = o.caseFirst || "false";
    var self = this;
    this.compare = function(a, b) {
      return self._cmp(String(a), String(b));
    };
  }
  Collator.prototype._cmp = function(sa, sb) {
    var a = sa, b = sb;
    if (this._ignorePunctuation) {
      a = a.replace(/[\p{P}\p{S}\s]+/gu, "");
      b = b.replace(/[\p{P}\p{S}\s]+/gu, "");
    }
    if (this._sensitivity === "base" || this._sensitivity === "accent") {
      a = a.toLowerCase();
      b = b.toLowerCase();
    }
    if (this._numeric) {
      var re = /(\d+|\D+)/g;
      var ta = a.match(re) || [], tb = b.match(re) || [];
      var len = Math.min(ta.length, tb.length);
      for (var i = 0; i < len; i++) {
        var pa = ta[i], pb = tb[i];
        if (pa === pb) continue;
        var na = /^\d+$/.test(pa), nb = /^\d+$/.test(pb);
        if (na && nb) {
          var diff = Number(pa) - Number(pb);
          if (diff !== 0) return diff < 0 ? -1 : 1;
        } else {
          return pa < pb ? -1 : 1;
        }
      }
      return ta.length === tb.length ? 0 : (ta.length < tb.length ? -1 : 1);
    }
    if (a === b) return 0;
    return a < b ? -1 : 1;
  };
  Collator.prototype.resolvedOptions = function() {
    return {
      locale: this._locale,
      usage: this._usage,
      sensitivity: this._sensitivity,
      ignorePunctuation: this._ignorePunctuation,
      numeric: this._numeric,
      caseFirst: this._caseFirst,
      collation: "default"
    };
  };
  Collator.supportedLocalesOf = DateTimeFormat.supportedLocalesOf;

  function PluralRules(locales, options) {
    if (!(this instanceof PluralRules)) return new PluralRules(locales, options);
    var o = options || {};
    this._locale = __normLocale(locales);
    this._type = o.type || "cardinal";
  }
  PluralRules.prototype.select = function(n) {
    var v = Math.abs(Number(n));
    if (this._type === "ordinal") {
      var mod10 = v % 10, mod100 = v % 100;
      if (mod10 === 1 && mod100 !== 11) return "one";
      if (mod10 === 2 && mod100 !== 12) return "two";
      if (mod10 === 3 && mod100 !== 13) return "few";
      return "other";
    }
    return v === 1 ? "one" : "other";
  };
  PluralRules.prototype.resolvedOptions = function() {
    return {
      locale: this._locale,
      type: this._type,
      pluralCategories: this._type === "ordinal" ? ["one", "two", "few", "other"] : ["one", "other"]
    };
  };
  PluralRules.supportedLocalesOf = DateTimeFormat.supportedLocalesOf;

  function Segmenter(locales, options) {
    if (!(this instanceof Segmenter)) return new Segmenter(locales, options);
    var o = options || {};
    this._locale = __normLocale(locales);
    this._granularity = o.granularity || "grapheme";
  }
  Segmenter.prototype.segment = function(input) {
    var str = String(input === undefined ? "" : input);
    var gran = this._granularity;
    var list = [];
    if (gran === "word") {
      var reW = /([A-Za-z0-9_]+|[^\sA-Za-z0-9_]+|\s+)/g, mW;
      while ((mW = reW.exec(str)) !== null) {
        var segW = mW[0];
        list.push({
          segment: segW,
          index: mW.index,
          input: str,
          isWordLike: /[A-Za-z0-9_]/.test(segW)
        });
      }
    } else if (gran === "sentence") {
      var reS = /[^.!?]+[.!?]*\s*/g, mS;
      while ((mS = reS.exec(str)) !== null) {
        list.push({ segment: mS[0], index: mS.index, input: str });
      }
      if (list.length === 0 && str.length > 0) {
        list.push({ segment: str, index: 0, input: str });
      }
    } else {
      var chars = Array.from(str);
      var idx = 0;
      for (var i = 0; i < chars.length; i++) {
        var ch = chars[i];
        list.push({ segment: ch, index: idx, input: str });
        idx += ch.length;
      }
    }
    var obj = {
      containing: function(codeUnitIndex) {
        var pos = Number(codeUnitIndex);
        if (Number.isNaN(pos) || pos < 0 || pos >= str.length) return undefined;
        for (var k = 0; k < list.length; k++) {
          var it = list[k];
          if (pos >= it.index && pos < it.index + it.segment.length) return it;
        }
        return undefined;
      }
    };
    obj[Symbol.iterator] = function() {
      var cur = 0;
      return {
        next: function() {
          return cur < list.length ? { value: list[cur++], done: false } : { value: undefined, done: true };
        }
      };
    };
    return obj;
  };
  Segmenter.prototype.resolvedOptions = function() {
    return { locale: this._locale, granularity: this._granularity };
  };
  Segmenter.supportedLocalesOf = DateTimeFormat.supportedLocalesOf;

  globalThis.Intl = {
    DateTimeFormat: DateTimeFormat,
    NumberFormat: NumberFormat,
    Collator: Collator,
    PluralRules: PluralRules,
    Segmenter: Segmenter,
    getCanonicalLocales: function(locales) {
      if (locales === undefined) return [];
      var arr = Array.isArray(locales) ? locales : [locales];
      var out = [];
      for (var i = 0; i < arr.length; i++) {
        var c = __normLocale(arr[i]);
        if (out.indexOf(c) === -1) out.push(c);
      }
      return out;
    },
    supportedValuesOf: function(key) {
      var k = String(key);
      if (k === "calendar") return ["gregory", "iso8601"];
      if (k === "collation") return ["default"];
      if (k === "currency") return ["AUD", "CAD", "EUR", "GBP", "ILS", "JPY", "USD"];
      if (k === "numberingSystem") return ["latn"];
      if (k === "timeZone") return ["UTC"];
      if (k === "unit") return ["byte", "day", "hour", "kilobyte", "megabyte", "millisecond", "minute", "month", "percent", "second", "week", "year"];
      throw new RangeError("Invalid key : " + k);
    }
  };

  Date.prototype.toLocaleDateString = function(locales, options) {
    return new DateTimeFormat(locales, options).format(this);
  };
  Date.prototype.toLocaleTimeString = function(locales, options) {
    var o = Object.assign({ hour: "numeric", minute: "2-digit", second: "2-digit" }, options || {});
    return new DateTimeFormat(locales, o).format(this);
  };
  Date.prototype.toLocaleString = function(locales, options) {
    var o = Object.assign({ year: "numeric", month: "numeric", day: "numeric", hour: "numeric", minute: "2-digit", second: "2-digit" }, options || {});
    return new DateTimeFormat(locales, o).format(this);
  };
  Number.prototype.toLocaleString = function(locales, options) {
    return new NumberFormat(locales, options).format(this);
  };
  String.prototype.localeCompare = function(that, locales, options) {
    return new Collator(locales, options).compare(String(this), String(that));
  };
})();
