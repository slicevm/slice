// Slice (Nexus-JS) Lazy Prelude Shard 1: WinterTC URL, Headers, Request, Response (`01_web_fetch_url.js`)
(function() {
  function __uspD(s) {
    if (s.indexOf("+") === -1 && s.indexOf("%") === -1) return s;
    var r = s.indexOf("+") === -1 ? s : s.split("+").join(" ");
    if (r.indexOf("%") === -1) return r;
    try { return decodeURIComponent(r); } catch (_e) { return r; }
  }

  function __uspE(s) {
    var str = String(s);
    var e = encodeURIComponent(str);
    return str.indexOf(" ") === -1 ? e : e.split("%20").join("+");
  }

  function URLSearchParams(init, u) {
    this._p = [];
    this._u = u || null;
    if (!init) return;
    if (typeof init === "string") {
      var q = init.charCodeAt(0) === 63 ? init.slice(1) : init;
      if (q.length > 0) {
        var ps = q.split("&");
        for (var i = 0; i < ps.length; i++) {
          var sg = ps[i];
          if (!sg) continue;
          var eq = sg.indexOf("=");
          if (eq === -1) this._p.push([__uspD(sg), ""]);
          else this._p.push([__uspD(sg.slice(0, eq)), __uspD(sg.slice(eq + 1))]);
        }
      }
    } else if (Array.isArray(init)) {
      for (var j = 0; j < init.length; j++) {
        var pr = init[j];
        if (pr && pr.length >= 2) this._p.push([String(pr[0]), String(pr[1])]);
      }
    } else if (typeof init === "object") {
      if (Array.isArray(init._p)) {
        for (var m = 0; m < init._p.length; m++) this._p.push([init._p[m][0], init._p[m][1]]);
      } else {
        var ks = Object.keys(init);
        for (var k = 0; k < ks.length; k++) this._p.push([ks[k], String(init[ks[k]])]);
      }
    }
  }
  URLSearchParams.prototype._sync = function() {
    if (this._u) this._u._syncSearch();
  };
  URLSearchParams.prototype.get = function(n) {
    var k = String(n);
    for (var i = 0; i < this._p.length; i++) {
      if (this._p[i][0] === k) return this._p[i][1];
    }
    return null;
  };
  URLSearchParams.prototype.getAll = function(n) {
    var k = String(n), r = [];
    for (var i = 0; i < this._p.length; i++) {
      if (this._p[i][0] === k) r.push(this._p[i][1]);
    }
    return r;
  };
  URLSearchParams.prototype.has = function(n) {
    var k = String(n);
    for (var i = 0; i < this._p.length; i++) {
      if (this._p[i][0] === k) return true;
    }
    return false;
  };
  URLSearchParams.prototype.set = function(n, v) {
    var k = String(n), s = String(v), p = this._p, len = p.length, fi = -1;
    for (var i = 0; i < len; i++) {
      if (p[i][0] === k) {
        if (fi === -1) {
          p[i][1] = s;
          fi = i;
        } else {
          p.splice(i, 1);
          i--;
          len--;
        }
      }
    }
    if (fi === -1) p.push([k, s]);
    if (this._u) this._u._syncSearch();
  };
  URLSearchParams.prototype.append = function(n, v) {
    this._p.push([String(n), String(v)]);
    if (this._u) this._u._syncSearch();
  };
  URLSearchParams.prototype.delete = function(n) {
    var k = String(n), nx = [];
    for (var i = 0; i < this._p.length; i++) {
      if (this._p[i][0] !== k) nx.push(this._p[i]);
    }
    this._p = nx;
    if (this._u) this._u._syncSearch();
  };
  URLSearchParams.prototype.sort = function() {
    this._p.sort(function(a, b) {
      return a[0] < b[0] ? -1 : (a[0] > b[0] ? 1 : 0);
    });
    if (this._u) this._u._syncSearch();
  };
  URLSearchParams.prototype.forEach = function(cb, ta) {
    for (var i = 0; i < this._p.length; i++) {
      cb.call(ta, this._p[i][1], this._p[i][0], this);
    }
  };
  URLSearchParams.prototype.entries = function() {
    return this._p.slice();
  };
  URLSearchParams.prototype.keys = function() {
    var r = [];
    for (var i = 0; i < this._p.length; i++) r.push(this._p[i][0]);
    return r;
  };
  URLSearchParams.prototype.values = function() {
    var r = [];
    for (var i = 0; i < this._p.length; i++) r.push(this._p[i][1]);
    return r;
  };
  URLSearchParams.prototype.toString = function() {
    var len = this._p.length;
    if (len === 0) return "";
    var s = "";
    for (var i = 0; i < len; i++) {
      if (i > 0) s += "&";
      s += __uspE(this._p[i][0]) + "=" + __uspE(this._p[i][1]);
    }
    return s;
  };
  if (typeof Symbol !== "undefined" && Symbol.iterator) {
    URLSearchParams.prototype[Symbol.iterator] = function() {
      return this._p[Symbol.iterator]();
    };
  }
  Object.defineProperty(URLSearchParams.prototype, "size", {
    get: function() { return this._p.length; }
  });

  function __normP(p) {
    if (!p || p === "/") return "/";
    if (p.indexOf("/.") === -1 && p.indexOf("//") === -1) return p;
    var segs = p.split("/"), out = [];
    for (var i = 0; i < segs.length; i++) {
      var s = segs[i];
      if (!s || s === ".") continue;
      if (s === "..") out.pop();
      else out.push(s);
    }
    return "/" + out.join("/") + (p.length > 1 && p.charCodeAt(p.length - 1) === 47 && out.length > 0 ? "/" : "");
  }

  function URL(input, base) {
    var raw = input instanceof URL ? input.href : String(input).trim();
    if (base !== undefined && raw.indexOf("https://") !== 0 && raw.indexOf("http://") !== 0 && raw.indexOf("://") === -1) {
      var b = base instanceof URL ? base : new URL(String(base));
      if (raw.indexOf("//") === 0) raw = b.protocol + raw;
      else if (raw.charCodeAt(0) === 47) raw = b.origin + raw;
      else if (raw.charCodeAt(0) === 63) raw = b.origin + b.pathname + raw;
      else if (raw.charCodeAt(0) === 35) raw = b.origin + b.pathname + b.search + raw;
      else {
        var dir = b.pathname.slice(0, b.pathname.lastIndexOf("/") + 1) || "/";
        raw = b.origin + dir + raw;
      }
    }
    var hi = raw.indexOf("#"), hash = "";
    if (hi !== -1) {
      hash = raw.slice(hi);
      raw = raw.slice(0, hi);
    }
    var qi = raw.indexOf("?"), search = "";
    if (qi !== -1) {
      search = raw.slice(qi);
      raw = raw.slice(0, qi);
    }
    var si = raw.indexOf("://");
    if (si <= 0) throw new TypeError("Invalid URL: " + input);
    this.protocol = raw.slice(0, si + 1).toLowerCase();
    var rest = raw.slice(si + 3);
    var pi = rest.indexOf("/");
    var authHost = pi === -1 ? rest : rest.slice(0, pi);
    var pathRaw = pi === -1 ? "/" : rest.slice(pi);
    var ai = authHost.indexOf("@"), hostPort = authHost;
    this.username = "";
    this.password = "";
    if (ai !== -1) {
      var up = authHost.slice(0, ai), ci = up.indexOf(":");
      this.username = ci === -1 ? up : up.slice(0, ci);
      this.password = ci === -1 ? "" : up.slice(ci + 1);
      hostPort = authHost.slice(ai + 1);
    }
    var pci = hostPort.lastIndexOf(":");
    if (pci !== -1 && hostPort.indexOf("]") < pci) {
      this.hostname = hostPort.slice(0, pci).toLowerCase();
      var prt = hostPort.slice(pci + 1);
      if ((this.protocol === "http:" && prt === "80") || (this.protocol === "https:" && prt === "443")) prt = "";
      this.port = prt;
    } else {
      this.hostname = hostPort.toLowerCase();
      this.port = "";
    }
    this.host = this.port ? this.hostname + ":" + this.port : this.hostname;
    this.origin = this.protocol + "//" + this.host;
    this.pathname = __normP(pathRaw);
    this.hash = hash === "#" ? "" : hash;
    this.search = search === "?" ? "" : search;
    this.href = this.origin + this.pathname + this.search + this.hash;
    this._sp = null;
  }
  Object.defineProperty(URL.prototype, "searchParams", {
    get: function() {
      return this._sp || (this._sp = new URLSearchParams(this.search, this));
    }
  });
  URL.prototype._syncSearch = function() {
    if (!this._sp || this._sp._p.length === 0) {
      this.search = "";
      this.href = this.origin + this.pathname + this.hash;
      return;
    }
    var qs = this._sp.toString();
    this.search = qs ? "?" + qs : "";
    this.href = this.origin + this.pathname + this.search + this.hash;
  };
  URL.prototype.toString = URL.prototype.toJSON = function() {
    return this.href;
  };
  URL.canParse = function(url, base) {
    try {
      new URL(url, base);
      return true;
    } catch (_e) {
      return false;
    }
  };
  var __blobUrlSeq = 1;
  globalThis.__slice_blob_urls = globalThis.__slice_blob_urls || Object.create(null);
  URL.createObjectURL = function(blob) {
    var id = "blob:slice/" + (__blobUrlSeq++);
    var txt = "";
    if (blob && blob._u8 instanceof Uint8Array) {
      txt = __slice_text_decode(blob._u8, "utf-8", false, false);
    } else if (blob !== undefined && blob !== null) {
      txt = String(blob);
    }
    globalThis.__slice_blob_urls[id] = txt;
    return id;
  };
  URL.revokeObjectURL = function(url) {
    if (globalThis.__slice_blob_urls) delete globalThis.__slice_blob_urls[String(url)];
  };

  function __escRe(s) {
    return s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  }

  function __compileUrlPat(pat, sep, ignoreCase) {
    var p = String(pat === undefined ? "*" : pat);
    if (p === "*") {
      return { re: new RegExp("^(.*)$", ignoreCase ? "i" : ""), keys: ["0"], hasRx: false };
    }
    var keys = [], hasRx = false, rx = "^", i = 0, n = p.length, anonIdx = 0;
    var segCh = sep ? "[^" + __escRe(sep) + "]" : ".";
    while (i < n) {
      var ch = p.charAt(i);
      if (ch === "\\") {
        if (i + 1 < n) {
          rx += __escRe(p.charAt(i + 1));
          i += 2;
        } else {
          rx += "\\\\";
          i++;
        }
        continue;
      }
      if (ch === "*") {
        var prevSlash = sep && i > 0 && p.charAt(i - 1) === sep && rx.slice(-__escRe(sep).length) === __escRe(sep);
        keys.push(String(anonIdx++));
        if (prevSlash && i + 1 === n) {
          rx = rx.slice(0, -__escRe(sep).length) + "(?:" + __escRe(sep) + "(.*))?";
        } else {
          rx += "(.*)";
        }
        i++;
        continue;
      }
      if (ch === ":") {
        var j = i + 1;
        while (j < n && /[a-zA-Z0-9_]/.test(p.charAt(j))) j++;
        var name = p.slice(i + 1, j);
        if (!name) throw new TypeError("Invalid URLPattern parameter at index " + i);
        keys.push(name);
        var customRe = null;
        if (j < n && p.charAt(j) === "(") {
          var depth = 1, k = j + 1;
          while (k < n && depth > 0) {
            if (p.charAt(k) === "\\") { k += 2; continue; }
            if (p.charAt(k) === "(") depth++;
            else if (p.charAt(k) === ")") depth--;
            k++;
          }
          if (depth !== 0) throw new TypeError("Unclosed regex group in URLPattern");
          customRe = p.slice(j + 1, k - 1);
          hasRx = true;
          j = k;
        }
        var mod = j < n && (p.charAt(j) === "?" || p.charAt(j) === "+" || p.charAt(j) === "*") ? p.charAt(j++) : "";
        var baseUnit = customRe ? "(?:" + customRe + ")" : segCh + "+";
        var hasLeadingSep = sep && rx.length > 1 && rx.slice(-__escRe(sep).length) === __escRe(sep);
        if (mod === "?") {
          if (hasLeadingSep) {
            rx = rx.slice(0, -__escRe(sep).length) + "(?:" + __escRe(sep) + "(" + baseUnit + "))?";
          } else {
            rx += "(" + baseUnit + ")?";
          }
        } else if (mod === "+") {
          var sepEsc = sep ? __escRe(sep) : "";
          rx += "(" + baseUnit + "(?:" + sepEsc + baseUnit + ")*)";
        } else if (mod === "*") {
          var sepEsc2 = sep ? __escRe(sep) : "";
          if (hasLeadingSep) {
            rx = rx.slice(0, -__escRe(sep).length) + "(?:" + sepEsc2 + "(" + baseUnit + "(?:" + sepEsc2 + baseUnit + ")*))?";
          } else {
            rx += "(" + (customRe ? "(?:" + customRe + ")*" : ".*") + ")";
          }
        } else {
          rx += "(" + baseUnit + ")";
        }
        i = j;
        continue;
      }
      if (ch === "(") {
        var d2 = 1, k2 = i + 1;
        while (k2 < n && d2 > 0) {
          if (p.charAt(k2) === "\\") { k2 += 2; continue; }
          if (p.charAt(k2) === "(") d2++;
          else if (p.charAt(k2) === ")") d2--;
          k2++;
        }
        if (d2 !== 0) throw new TypeError("Unclosed group in URLPattern");
        var inner = p.slice(i + 1, k2 - 1);
        var mod2 = k2 < n && (p.charAt(k2) === "?" || p.charAt(k2) === "+" || p.charAt(k2) === "*") ? p.charAt(k2++) : "";
        keys.push(String(anonIdx++));
        hasRx = true;
        rx += "((?:" + inner + "))" + mod2;
        i = k2;
        continue;
      }
      if (ch === "{") {
        var closeIdx = p.indexOf("}", i + 1);
        if (closeIdx === -1) throw new TypeError("Unclosed brace in URLPattern");
        var braceInner = p.slice(i + 1, closeIdx);
        var mod3 = closeIdx + 1 < n && (p.charAt(closeIdx + 1) === "?" || p.charAt(closeIdx + 1) === "+" || p.charAt(closeIdx + 1) === "*") ? p.charAt(closeIdx + 1) : "";
        var sub = __compileUrlPat(braceInner, sep, ignoreCase);
        for (var sk = 0; sk < sub.keys.length; sk++) {
          keys.push(sub.keys[sk] === "0" ? String(anonIdx++) : sub.keys[sk]);
        }
        if (sub.hasRx) hasRx = true;
        var subSrc = sub.re.source.slice(1, -1);
        rx += "(?:" + subSrc + ")" + mod3;
        i = closeIdx + 1 + (mod3 ? 1 : 0);
        continue;
      }
      rx += __escRe(ch);
      i++;
    }
    rx += "$";
    return { re: new RegExp(rx, ignoreCase ? "i" : ""), keys: keys, hasRx: hasRx };
  }

  function __findUrlPatQueryIdx(s) {
    var paren = 0, brace = 0, inParam = false;
    for (var i = 0; i < s.length; i++) {
      var c = s.charAt(i);
      if (c === "\\") { i++; continue; }
      if (c === "(") { paren++; inParam = false; continue; }
      if (c === ")") {
        if (paren > 0) paren--;
        if (paren === 0 && i + 1 < s.length && s.charAt(i + 1) === "?") { i++; continue; }
        continue;
      }
      if (paren > 0) continue;
      if (c === "{") { brace++; inParam = false; continue; }
      if (c === "}") {
        if (brace > 0) brace--;
        if (brace === 0 && i + 1 < s.length && s.charAt(i + 1) === "?") { i++; continue; }
        continue;
      }
      if (brace > 0) continue;
      if (c === ":" && !(i + 2 < s.length && s.slice(i, i + 3) === "://")) {
        inParam = true;
        continue;
      }
      if (inParam) {
        if (/[a-zA-Z0-9_]/.test(c)) continue;
        inParam = false;
        if (c === "?") continue;
      }
      if (c === "?") return i;
    }
    return -1;
  }

  function __parseUrlPatternStr(str, baseURL) {
    var s = String(str).trim();
    var init = {
      protocol: "*",
      username: "*",
      password: "*",
      hostname: "*",
      port: "*",
      pathname: "*",
      search: "*",
      hash: "*"
    };
    if (baseURL !== undefined) {
      var b = new URL(String(baseURL));
      init.protocol = b.protocol.replace(/:$/, "");
      init.username = b.username || "*";
      init.password = b.password || "*";
      init.hostname = b.hostname;
      init.port = b.port || "";
      init.pathname = b.pathname;
    }
    var hi = s.indexOf("#");
    if (hi !== -1) {
      init.hash = s.slice(hi + 1);
      s = s.slice(0, hi);
    }
    var qi = __findUrlPatQueryIdx(s);
    if (qi !== -1) {
      init.search = s.slice(qi + 1);
      s = s.slice(0, qi);
    }
    var si = s.indexOf("://");
    if (si !== -1) {
      init.protocol = s.slice(0, si);
      var rest = s.slice(si + 3);
      var sl = rest.indexOf("/");
      var auth = sl === -1 ? rest : rest.slice(0, sl);
      init.pathname = sl === -1 ? "/" : rest.slice(sl);
      var at = auth.indexOf("@");
      if (at !== -1) {
        var ui = auth.slice(0, at);
        auth = auth.slice(at + 1);
        var cl = ui.indexOf(":");
        if (cl === -1) {
          init.username = ui;
        } else {
          init.username = ui.slice(0, cl);
          init.password = ui.slice(cl + 1);
        }
      }
      var pCol = auth.lastIndexOf(":");
      if (pCol !== -1 && auth.indexOf("]") < pCol) {
        init.hostname = auth.slice(0, pCol);
        init.port = auth.slice(pCol + 1);
      } else {
        init.hostname = auth;
        init.port = "*";
      }
    } else if (s.length > 0) {
      if (s.charCodeAt(0) === 47) {
        init.pathname = s;
      } else if (baseURL !== undefined) {
        var dir = init.pathname.slice(0, init.pathname.lastIndexOf("/") + 1) || "/";
        init.pathname = dir + s;
      } else {
        init.pathname = "/" + s;
      }
    }
    return init;
  }

  function URLPattern(input, baseURL, options) {
    if (!(this instanceof URLPattern)) {
      throw new TypeError("Failed to construct 'URLPattern': Please use the 'new' operator");
    }
    var opts = options;
    var base = baseURL;
    if (base && typeof base === "object" && options === undefined) {
      opts = base;
      base = undefined;
    }
    var ignoreCase = Boolean(opts && opts.ignoreCase);
    var init;
    if (input === undefined) {
      init = { protocol: "*", username: "*", password: "*", hostname: "*", port: "*", pathname: "*", search: "*", hash: "*" };
    } else if (typeof input === "string" || input instanceof URL) {
      init = __parseUrlPatternStr(input instanceof URL ? input.href : input, base);
    } else if (typeof input === "object" && input !== null) {
      if (base !== undefined) {
        throw new TypeError("Failed to construct 'URLPattern': Second argument cannot be a baseURL when first argument is a dictionary");
      }
      var bObj = input.baseURL ? new URL(String(input.baseURL)) : null;
      init = {
        protocol: input.protocol !== undefined ? String(input.protocol).replace(/:$/, "") : (bObj ? bObj.protocol.replace(/:$/, "") : "*"),
        username: input.username !== undefined ? String(input.username) : (bObj && bObj.username ? bObj.username : "*"),
        password: input.password !== undefined ? String(input.password) : (bObj && bObj.password ? bObj.password : "*"),
        hostname: input.hostname !== undefined ? String(input.hostname) : (bObj ? bObj.hostname : "*"),
        port: input.port !== undefined ? String(input.port) : (bObj ? bObj.port : "*"),
        pathname: input.pathname !== undefined ? String(input.pathname) : (bObj ? bObj.pathname : "*"),
        search: input.search !== undefined ? String(input.search).replace(/^\?/, "") : "*",
        hash: input.hash !== undefined ? String(input.hash).replace(/^#/, "") : "*"
      };
    } else {
      throw new TypeError("Failed to construct 'URLPattern': Invalid input");
    }
    this.protocol = init.protocol;
    this.username = init.username;
    this.password = init.password;
    this.hostname = init.hostname;
    this.port = init.port;
    this.pathname = init.pathname;
    this.search = init.search;
    this.hash = init.hash;
    this._c = {
      protocol: __compileUrlPat(this.protocol, "", true),
      username: __compileUrlPat(this.username, "", ignoreCase),
      password: __compileUrlPat(this.password, "", ignoreCase),
      hostname: __compileUrlPat(this.hostname, ".", true),
      port: __compileUrlPat(this.port, "", false),
      pathname: __compileUrlPat(this.pathname, "/", ignoreCase),
      search: __compileUrlPat(this.search, "", ignoreCase),
      hash: __compileUrlPat(this.hash, "", ignoreCase)
    };
    this.hasRegExpGroups = Boolean(
      this._c.protocol.hasRx || this._c.username.hasRx || this._c.password.hasRx ||
      this._c.hostname.hasRx || this._c.port.hasRx || this._c.pathname.hasRx ||
      this._c.search.hasRx || this._c.hash.hasRx
    );
  }

  URLPattern.prototype._normTarget = function(input, baseURL) {
    if (typeof input === "string" || input instanceof URL) {
      var raw = input instanceof URL ? input.href : String(input);
      try {
        var u = baseURL !== undefined
          ? new URL(raw, baseURL)
          : (raw.indexOf("://") === -1 && raw.charCodeAt(0) === 47 && this.protocol === "*" && this.hostname === "*"
              ? new URL(raw, "http://localhost")
              : new URL(raw));
        return {
          inputs: baseURL !== undefined ? [String(input), String(baseURL)] : [String(input)],
          protocol: u.protocol.replace(/:$/, ""),
          username: u.username || "",
          password: u.password || "",
          hostname: u.hostname || "",
          port: u.port || "",
          pathname: u.pathname || "/",
          search: u.search ? u.search.slice(1) : "",
          hash: u.hash ? u.hash.slice(1) : ""
        };
      } catch (_e) {
        return null;
      }
    }
    if (input && typeof input === "object") {
      if (baseURL !== undefined) {
        throw new TypeError("URLPattern: baseURL is not allowed when input is an object");
      }
      if (typeof input.url === "string" && input.pathname === undefined) {
        return this._normTarget(input.url);
      }
      var b = input.baseURL ? new URL(String(input.baseURL)) : null;
      return {
        inputs: [input],
        protocol: input.protocol !== undefined ? String(input.protocol).replace(/:$/, "") : (b ? b.protocol.replace(/:$/, "") : ""),
        username: input.username !== undefined ? String(input.username) : (b ? b.username : ""),
        password: input.password !== undefined ? String(input.password) : (b ? b.password : ""),
        hostname: input.hostname !== undefined ? String(input.hostname) : (b ? b.hostname : ""),
        port: input.port !== undefined ? String(input.port) : (b ? b.port : ""),
        pathname: input.pathname !== undefined ? String(input.pathname) : (b ? b.pathname : "/"),
        search: input.search !== undefined ? String(input.search).replace(/^\?/, "") : (b && b.search ? b.search.slice(1) : ""),
        hash: input.hash !== undefined ? String(input.hash).replace(/^#/, "") : (b && b.hash ? b.hash.slice(1) : "")
      };
    }
    return null;
  };

  URLPattern.prototype.exec = function(input, baseURL) {
    var t = this._normTarget(input, baseURL);
    if (!t) return null;
    var parts = ["protocol", "username", "password", "hostname", "port", "pathname", "search", "hash"];
    var res = { inputs: t.inputs };
    for (var i = 0; i < 8; i++) {
      var k = parts[i];
      var comp = this._c[k];
      var val = t[k];
      var m = comp.re.exec(val);
      if (!m) return null;
      var groups = {};
      for (var j = 0; j < comp.keys.length; j++) {
        groups[comp.keys[j]] = m[j + 1];
      }
      res[k] = { input: val, groups: groups };
    }
    return res;
  };

  URLPattern.prototype.test = function(input, baseURL) {
    return this.exec(input, baseURL) !== null;
  };

  function Headers(init) {
    if (init === "__SLICE_LIVE_REQ__") {
      this._live = true;
      this._m = null;
      return;
    }
    this._live = false;
    this._m = Object.create(null);
    if (!init) return;
    if (init instanceof Headers) {
      if (init._m) {
        var ik = Object.keys(init._m);
        for (var n = 0; n < ik.length; n++) this._m[ik[n]] = init._m[ik[n]];
      } else {
        var es = init.entries();
        for (var i = 0; i < es.length; i++) this._m[es[i][0]] = es[i][1];
      }
    } else if (Array.isArray(init)) {
      for (var j = 0; j < init.length; j++) {
        if (init[j] && init[j].length >= 2) this.append(init[j][0], init[j][1]);
      }
    } else if (typeof init === "object") {
      var ks = Object.keys(init);
      for (var k = 0; k < ks.length; k++) {
        if (init[ks[k]] !== undefined) this._m[ks[k].toLowerCase()] = String(init[ks[k]]);
      }
    }
  }
  Headers.prototype._ensure = function() {
    if (this._live && !this._m) {
      this._m = Object.create(null);
      var raw = JSON.parse(__slice_http_headers_all());
      for (var i = 0; i < raw.length; i++) {
        var k = raw[i][0].toLowerCase();
        this._m[k] = this._m[k] ? this._m[k] + ", " + raw[i][1] : raw[i][1];
      }
      this._live = false;
    }
  };
  Headers.prototype.get = function(n) {
    if (typeof n === "string" && this._m && n in this._m) return this._m[n];
    var k = String(n).toLowerCase();
    if (this._m && k in this._m) return this._m[k];
    if (this._live) return __slice_http_header(k);
    return null;
  };
  Headers.prototype.has = function(n) {
    return this.get(n) !== null;
  };
  Headers.prototype.set = function(n, v) {
    this._ensure();
    this._m[String(n).toLowerCase()] = String(v);
  };
  Headers.prototype.append = function(n, v) {
    this._ensure();
    var k = String(n).toLowerCase(), s = String(v);
    this._m[k] = k in this._m ? this._m[k] + ", " + s : s;
  };
  Headers.prototype.delete = function(n) {
    this._ensure();
    delete this._m[String(n).toLowerCase()];
  };
  Headers.prototype.entries = function() {
    this._ensure();
    var ks = Object.keys(this._m).sort(), r = [];
    for (var i = 0; i < ks.length; i++) r.push([ks[i], this._m[ks[i]]]);
    return r;
  };
  Headers.prototype.keys = function() {
    this._ensure();
    return Object.keys(this._m).sort();
  };
  Headers.prototype.values = function() {
    var es = this.entries(), r = [];
    for (var i = 0; i < es.length; i++) r.push(es[i][1]);
    return r;
  };
  Headers.prototype.forEach = function(cb, ta) {
    this._ensure();
    var m = this._m, ks = Object.keys(m);
    for (var i = 0; i < ks.length; i++) {
      var k = ks[i];
      cb.call(ta, m[k], k, this);
    }
  };
  if (typeof Symbol !== "undefined" && Symbol.iterator) {
    Headers.prototype[Symbol.iterator] = function() {
      return this.entries()[Symbol.iterator]();
    };
  }

  function __extractBodyBytes(obj) {
    if (obj._u8 instanceof Uint8Array) return new Uint8Array(obj._u8);
    var s = obj._body || (obj._data !== undefined ? JSON.stringify(obj._data) : "");
    return __slice_text_encode(s);
  }

  function __bodyToFormData(obj) {
    if (obj._fd && obj._fd._isFormData) return obj._fd;
    var fd = new globalThis.FormData();
    var usp = new URLSearchParams(obj._body || "");
    var es = usp.entries();
    for (var i = 0; i < es.length; i++) fd.append(es[i][0], es[i][1]);
    return fd;
  }

  function __normHttpBody(rawBody, headers) {
    if (rawBody instanceof URLSearchParams) {
      if (headers && !headers.has("content-type")) {
        headers.set("content-type", "application/x-www-form-urlencoded;charset=UTF-8");
      }
      return { str: rawBody.toString(), u8: null, fd: null };
    }
    if (typeof rawBody === "object") {
      if (rawBody._isFormData) {
        if (headers && !headers.has("content-type")) {
          headers.set("content-type", "multipart/form-data; boundary=----SliceFormBoundary");
        }
        var pairs = [], es = rawBody.entries();
        for (var i = 0; i < es.length; i++) {
          var v = es[i][1];
          pairs.push(
            encodeURIComponent(es[i][0]) +
            "=" +
            encodeURIComponent(v && v._isBlob ? __slice_text_decode(v._u8, "utf-8", false, false) : String(v))
          );
        }
        return { str: pairs.join("&"), u8: null, fd: rawBody };
      }
      if (rawBody._isBlob) {
        if (rawBody.type && headers && !headers.has("content-type")) {
          headers.set("content-type", rawBody.type);
        }
        var u8b = new Uint8Array(rawBody._u8);
        return { str: __slice_text_decode(u8b, "utf-8", false, false), u8: u8b, fd: null };
      }
      if (
        rawBody instanceof Uint8Array ||
        rawBody instanceof ArrayBuffer ||
        (rawBody.buffer instanceof ArrayBuffer)
      ) {
        var u8 = new Uint8Array(__slice_toU8(rawBody));
        return { str: __slice_text_decode(u8, "utf-8", false, false), u8: u8, fd: null };
      }
      if (typeof rawBody._toText === "function") {
        var txt = rawBody._toText();
        return {
          str: txt,
          u8: typeof rawBody._toBytes === "function" ? rawBody._toBytes() : __slice_text_encode(txt),
          fd: null
        };
      }
    }
    return { str: JSON.stringify(rawBody), u8: null, fd: null };
  }

  function Request(input, init) {
    if (input === "__SLICE_LIVE_REQ__") {
      this.method = __slice_http_method() || "GET";
      var h = __slice_http_header("host") || "localhost";
      var p = __slice_http_path() || "/";
      var q = __slice_http_query();
      this.url = "http://" + h + p + (q ? "?" + q : "");
      this.headers = new Headers("__SLICE_LIVE_REQ__");
      this._body = __slice_http_body() || "";
      this._u8 = null;
      this._fd = null;
      this.redirect = "follow";
      this.signal = null;
      this.bodyUsed = false;
      return;
    }
    if (!init && typeof input === "string") {
      this.url = input;
      this.method = "GET";
      this.headers = new Headers();
      this._body = "";
      this._u8 = null;
      this._fd = null;
      this.redirect = "follow";
      this.signal = null;
      this.bodyUsed = false;
      return;
    }
    var o = init || {}, isReq = input instanceof Request;
    this.url = isReq ? input.url : (input instanceof URL ? input.href : String(input));
    this.method = o.method ? String(o.method).toUpperCase() : (isReq ? input.method : "GET");
    this.headers = o.headers instanceof Headers ? o.headers : new Headers(o.headers || (isReq ? input.headers : undefined));
    if (o.body !== undefined && o.body !== null) {
      if (typeof o.body === "string") {
        this._body = o.body;
        this._u8 = null;
        this._fd = null;
      } else {
        var nb = __normHttpBody(o.body, this.headers);
        this._body = nb.str;
        this._u8 = nb.u8;
        this._fd = nb.fd;
      }
    } else if (isReq) {
      this._body = input._body;
      this._u8 = input._u8 ? new Uint8Array(input._u8) : null;
      this._fd = input._fd || null;
    } else {
      this._body = "";
      this._u8 = null;
      this._fd = null;
    }
    this.redirect = o.redirect ? String(o.redirect) : (isReq && input.redirect ? input.redirect : "follow");
    this.signal = o.signal || (isReq && input.signal ? input.signal : null);
    this.bodyUsed = false;
  }

  function Response(body, init) {
    var o = init || {};
    this.status = o.status !== undefined ? Number(o.status) : 200;
    this.statusText = o.statusText !== undefined ? String(o.statusText) : (this.status === 200 ? "OK" : "");
    this.ok = this.status >= 200 && this.status < 300;
    this.headers = o.headers instanceof Headers ? o.headers : new Headers(o.headers);
    this._data = undefined;
    if (body === undefined || body === null) {
      this._body = "";
      this._u8 = null;
      this._fd = null;
    } else if (typeof body === "string") {
      this._body = body;
      this._u8 = null;
      this._fd = null;
    } else {
      var nb = __normHttpBody(body, this.headers);
      this._body = nb.str;
      this._u8 = nb.u8;
      this._fd = nb.fd;
    }
    this._ct = (this.headers._m && this.headers._m["content-type"]) || "text/plain;charset=UTF-8";
    this._isSliceRes = true;
    this.bodyUsed = false;
  }

  var RP = Request.prototype, SP = Response.prototype;
  RP.text = SP.text = function() {
    this.bodyUsed = true;
    if (this._data !== undefined && !this._body) this._body = JSON.stringify(this._data);
    return Promise.resolve(this._body);
  };
  RP.json = SP.json = function() {
    this.bodyUsed = true;
    if (this._data !== undefined) return Promise.resolve(this._data);
    try { return Promise.resolve(JSON.parse(this._body)); } catch (e) { return Promise.reject(e); }
  };
  RP.textSync = SP.textSync = function() {
    this.bodyUsed = true;
    if (this._data !== undefined && !this._body) this._body = JSON.stringify(this._data);
    return this._body;
  };
  RP.jsonSync = SP.jsonSync = function() {
    this.bodyUsed = true;
    if (this._data !== undefined) return this._data;
    return JSON.parse(this._body);
  };
  RP.arrayBuffer = SP.arrayBuffer = function() {
    this.bodyUsed = true;
    return Promise.resolve(__extractBodyBytes(this).buffer);
  };
  RP.bytes = SP.bytes = function() {
    this.bodyUsed = true;
    return Promise.resolve(__extractBodyBytes(this));
  };
  RP.blob = SP.blob = function() {
    this.bodyUsed = true;
    if (!globalThis.Blob) __slice_ensure_shard(3);
    return Promise.resolve(new globalThis.Blob([__extractBodyBytes(this)], { type: this._ct || this.headers.get("content-type") || "" }));
  };
  RP.formData = SP.formData = function() {
    this.bodyUsed = true;
    if (!globalThis.FormData) __slice_ensure_shard(3);
    return Promise.resolve(__bodyToFormData(this));
  };
  RP.clone = function() {
    var r = new Request(this.url, {
      method: this.method,
      headers: new Headers(this.headers),
      body: this._body,
      redirect: this.redirect,
      signal: this.signal
    });
    if (this._u8) r._u8 = new Uint8Array(this._u8);
    if (this._fd) r._fd = this._fd;
    return r;
  };
  SP.clone = function() {
    if (this._data !== undefined && !this._body) this._body = JSON.stringify(this._data);
    var r = new Response(this._body, {
      status: this.status,
      statusText: this.statusText,
      headers: new Headers(this.headers)
    });
    if (this._u8) r._u8 = new Uint8Array(this._u8);
    if (this._fd) r._fd = this._fd;
    return r;
  };
  var __bodyDesc = {
    get: function() {
      if (!this._body && !this._u8 && this._data === undefined) return null;
      if (!globalThis.ReadableStream) __slice_ensure_shard(3);
      var b = __extractBodyBytes(this);
      return new globalThis.ReadableStream({
        start: function(c) {
          if (b.byteLength > 0) c.enqueue(b);
          c.close();
        }
      });
    }
  };
  Object.defineProperty(RP, "body", __bodyDesc);
  Object.defineProperty(SP, "body", __bodyDesc);
  Response.json = function(data, init) {
    var r = new Response(null, typeof init === "number" ? { status: init } : init);
    if (!r.headers._m["content-type"]) r.headers._m["content-type"] = "application/json";
    r._data = data;
    r._ct = r.headers._m["content-type"];
    return r;
  };
  Response.redirect = function(url, status) {
    var st = status || 302;
    return new Response("", { status: st, headers: { location: String(url) } });
  };
  Response.error = function() {
    var r = new Response("", { status: 0, statusText: "" });
    r.ok = false;
    r.type = "error";
    return r;
  };

  function __resHdrJson(h) {
    if (!h || !h._m) return "";
    var ks = Object.keys(h._m), out = [];
    for (var i = 0; i < ks.length; i++) {
      var k = ks[i];
      if (k !== "content-type" && k !== "content-length") out.push([k, h._m[k]]);
    }
    return out.length > 0 ? JSON.stringify(out) : "";
  }

  globalThis.URLSearchParams = URLSearchParams;
  globalThis.URL = URL;
  globalThis.URLPattern = URLPattern;
  globalThis.Headers = Headers;
  globalThis.Request = Request;
  globalThis.Response = Response;

  var __slice_web_ctx = {
    waitUntil: globalThis.Slice ? globalThis.Slice.waitUntil : function() {},
    passThroughOnException: function() {}
  };
  globalThis.__slice_make_web_request = function() {
    return new Request("__SLICE_LIVE_REQ__");
  };
  globalThis.__slice_dispatch_fetch = function(h) {
    if (!__slice_http_is_active()) return;
    var fn = typeof h.fetch === "function" ? h.fetch : (typeof h === "function" ? h : null);
    if (!fn) return;
    var res = fn.call(h, new Request("__SLICE_LIVE_REQ__"), globalThis.process.env, __slice_web_ctx);
    if (res) {
      if (res._isSliceRes) {
        var b = res._body || (res._data !== undefined ? JSON.stringify(res._data) : "");
        __slice_http_respond(res.status, b, res._ct, __resHdrJson(res.headers));
      } else if (typeof res.then === "function") {
        res.then(function(r) {
          if (r && r._isSliceRes) {
            var rb = r._body || (r._data !== undefined ? JSON.stringify(r._data) : "");
            __slice_http_respond(r.status, rb, r._ct, __resHdrJson(r.headers));
          }
        });
      }
    }
  };
})();
