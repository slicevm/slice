// Slice (Nexus-JS) Lazy Prelude Shard 1: WinterTC `URL` and `URLSearchParams` (`01_web_fetch_url.js`)
(function() {
  function __uspD(s) {
    if (s.indexOf("+") === -1 && s.indexOf("%") === -1) return s;
    return __slice_usp_decode(s);
  }

  function __uspE(s) {
    var str = typeof s === "string" ? s : String(s), n = str.length;
    if (n === 0) return "";
    if (n <= 64) {
      var safe = true;
      for (var i = 0; i < n; i++) {
        var c = str.charCodeAt(i);
        if (
          !(
            (c >= 97 && c <= 122) ||
            (c >= 65 && c <= 90) ||
            (c >= 48 && c <= 57) ||
            c === 45 ||
            c === 95 ||
            c === 46 ||
            c === 126
          )
        ) {
          safe = false;
          break;
        }
      }
      if (safe) return str;
    }
    return __slice_usp_encode(str);
  }

  function URLSearchParams(init, u) {
    this._u = u || null;
    this._raw = "";
    if (!init) {
      this._p = [];
      return;
    }
    if (typeof init === "string") {
      var q = init.charCodeAt(0) === 63 ? init.slice(1) : init;
      if (q.length === 0) {
        this._p = [];
      } else {
        this._raw = q;
        this._p = null;
      }
      return;
    }
    this._p = [];
    if (Array.isArray(init)) {
      for (var j = 0; j < init.length; j++) {
        var pr = init[j];
        if (pr && pr.length >= 2) this._p.push([String(pr[0]), String(pr[1])]);
      }
    } else if (typeof init === "object") {
      if (init._isUSP) {
        var sp = init._ensureP();
        for (var m = 0; m < sp.length; m++) this._p.push([sp[m][0], sp[m][1]]);
      } else {
        var ks = Object.keys(init);
        for (var k = 0; k < ks.length; k++) this._p.push([ks[k], String(init[ks[k]])]);
      }
    }
  }
  URLSearchParams.prototype._isUSP = true;
  URLSearchParams.prototype._ensureP = function() {
    var p = this._p;
    if (p !== null) return p;
    p = [];
    this._p = p;
    var q = this._raw;
    this._raw = "";
    if (q.length > 0) {
      var ps = q.split("&");
      for (var i = 0; i < ps.length; i++) {
        var sg = ps[i];
        if (!sg) continue;
        var eq = sg.indexOf("=");
        if (eq === -1) p.push([__uspD(sg), ""]);
        else p.push([__uspD(sg.slice(0, eq)), __uspD(sg.slice(eq + 1))]);
      }
    }
    return p;
  };
  URLSearchParams.prototype._sync = function() {
    if (this._u) this._u._syncSearch();
  };
  URLSearchParams.prototype.get = function(n) {
    var k = typeof n === "string" ? n : String(n);
    var p = this._p;
    if (p === null) {
      var q = this._raw;
      if (!q) return null;
      if (q.indexOf("+") === -1 && q.indexOf("%") === -1) {
        var qLen = q.length, kLen = k.length, pos = 0;
        while (pos < qLen) {
          var amp = q.indexOf("&", pos);
          var end = amp === -1 ? qLen : amp;
          var sgLen = end - pos;
          if (sgLen > 0) {
            if (sgLen > kLen && q.charCodeAt(pos + kLen) === 61 && q.startsWith(k, pos)) {
              return q.slice(pos + kLen + 1, end);
            }
            if (sgLen === kLen && q.startsWith(k, pos)) {
              return "";
            }
          }
          if (amp === -1) break;
          pos = amp + 1;
        }
        return null;
      }
      p = this._ensureP();
    }
    for (var i = 0; i < p.length; i++) {
      if (p[i][0] === k) return p[i][1];
    }
    return null;
  };
  URLSearchParams.prototype.getAll = function(n) {
    var k = typeof n === "string" ? n : String(n), p = this._ensureP(), r = [];
    for (var i = 0; i < p.length; i++) {
      if (p[i][0] === k) r.push(p[i][1]);
    }
    return r;
  };
  URLSearchParams.prototype.has = function(n) {
    if (this._p === null) return this.get(n) !== null;
    var k = typeof n === "string" ? n : String(n), p = this._p;
    for (var i = 0; i < p.length; i++) {
      if (p[i][0] === k) return true;
    }
    return false;
  };
  URLSearchParams.prototype.set = function(n, v) {
    var k = typeof n === "string" ? n : String(n);
    var s = typeof v === "string" ? v : String(v);
    var p = this._ensureP(), len = p.length, fi = -1;
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
    this._ensureP().push([typeof n === "string" ? n : String(n), typeof v === "string" ? v : String(v)]);
    if (this._u) this._u._syncSearch();
  };
  URLSearchParams.prototype.delete = function(n) {
    var k = typeof n === "string" ? n : String(n), p = this._ensureP(), nx = [];
    for (var i = 0; i < p.length; i++) {
      if (p[i][0] !== k) nx.push(p[i]);
    }
    this._p = nx;
    if (this._u) this._u._syncSearch();
  };
  URLSearchParams.prototype.sort = function() {
    this._ensureP().sort(function(a, b) {
      return a[0] < b[0] ? -1 : (a[0] > b[0] ? 1 : 0);
    });
    if (this._u) this._u._syncSearch();
  };
  URLSearchParams.prototype.forEach = function(cb, ta) {
    var p = this._ensureP();
    for (var i = 0; i < p.length; i++) {
      cb.call(ta, p[i][1], p[i][0], this);
    }
  };
  URLSearchParams.prototype.entries = function() {
    return this._ensureP().slice();
  };
  URLSearchParams.prototype.keys = function() {
    var p = this._ensureP(), r = [];
    for (var i = 0; i < p.length; i++) r.push(p[i][0]);
    return r;
  };
  URLSearchParams.prototype.values = function() {
    var p = this._ensureP(), r = [];
    for (var i = 0; i < p.length; i++) r.push(p[i][1]);
    return r;
  };
  URLSearchParams.prototype.toString = function() {
    var p = this._ensureP(), len = p.length;
    if (len === 0) return "";
    var s = "";
    for (var i = 0; i < len; i++) {
      if (i > 0) s += "&";
      s += __uspE(p[i][0]) + "=" + __uspE(p[i][1]);
    }
    return s;
  };
  if (typeof Symbol !== "undefined" && Symbol.iterator) {
    URLSearchParams.prototype[Symbol.iterator] = function() {
      return this._ensureP()[Symbol.iterator]();
    };
  }
  Object.defineProperty(URLSearchParams.prototype, "size", {
    get: function() { return this._ensureP().length; }
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

  function __hasNonSpecialScheme(s) {
    var ci = s.indexOf(":");
    if (ci <= 0) return false;
    var sl = s.indexOf("/");
    if (sl !== -1 && sl < ci) return false;
    if (!/^[a-zA-Z][a-zA-Z0-9+.-]*$/.test(s.slice(0, ci))) return false;
    var p = s.slice(0, ci + 1).toLowerCase();
    return p !== "http:" && p !== "https:" && p !== "ws:" && p !== "wss:" && p !== "file:" && p !== "ftp:";
  }

  var FastUrlProto;

  function __tryFastHttpUrl(s) {
    var len = s.length;
    if (len < 8 || s.charCodeAt(0) !== 104 || s.charCodeAt(len - 1) <= 32) return null;
    var rs = 0;
    if (s.charCodeAt(1) === 116 && s.charCodeAt(2) === 116 && s.charCodeAt(3) === 112) {
      if (s.charCodeAt(4) === 58 && s.charCodeAt(5) === 47 && s.charCodeAt(6) === 47) rs = 7;
      else if (s.charCodeAt(4) === 115 && s.charCodeAt(5) === 58 && s.charCodeAt(6) === 47 && s.charCodeAt(7) === 47) rs = 8;
    }
    if (rs === 0 || s.indexOf("#", rs) !== -1) return null;
    var qi = s.indexOf("?", rs);
    var pathEnd = qi === -1 ? len : qi;
    var pi = s.indexOf("/", rs);
    if (pi <= rs || pi > pathEnd) return null;
    for (var i = rs; i < pi; i++) {
      var c = s.charCodeAt(i);
      if (c <= 32 || c === 64 || (c >= 65 && c <= 90)) return null;
    }
    if (
      (rs === 7 && pi - rs > 3 && s.charCodeAt(pi - 3) === 58 && s.charCodeAt(pi - 2) === 56 && s.charCodeAt(pi - 1) === 48) ||
      (rs === 8 && pi - rs > 4 && s.charCodeAt(pi - 4) === 58 && s.charCodeAt(pi - 3) === 52 && s.charCodeAt(pi - 2) === 52 && s.charCodeAt(pi - 1) === 51)
    ) {
      return null;
    }
    if (pathEnd - pi > 1) {
      for (var p = pi; p < pathEnd - 1; p++) {
        if (s.charCodeAt(p) === 47) {
          var nx = s.charCodeAt(p + 1);
          if (nx === 47 || nx === 46) return null;
        }
      }
    }
    if (qi !== -1 && qi + 1 === len) return null;
    var u = Object.create(FastUrlProto);
    u._r = s;
    u._rs = rs;
    u._pi = pi;
    u._qi = qi;
    u._sp = null;
    return u;
  }

  function URL(input, base) {
    if (base === undefined && typeof input === "string") {
      var fastU = __tryFastHttpUrl(input);
      if (fastU !== null) return fastU;
    }
    var raw = (input && input._isSliceUrl) ? input.href : (typeof input === "string" ? input.trim() : String(input).trim());
    if (base !== undefined && raw.indexOf("https://") !== 0 && raw.indexOf("http://") !== 0 && raw.indexOf("://") === -1 && !__hasNonSpecialScheme(raw)) {
      if (typeof base === "string" && raw.charCodeAt(0) === 47 && raw.charCodeAt(1) !== 47) {
        var bsi = base.indexOf("://");
        if (bsi > 0 && base.indexOf("?", bsi + 3) === -1 && base.indexOf("#", bsi + 3) === -1) {
          var bsl = base.indexOf("/", bsi + 3);
          raw = (bsl === -1 ? base : base.slice(0, bsl)) + raw;
        } else {
          var b0 = new URL(base);
          raw = b0.origin + raw;
        }
      } else {
        var b = (base && base._isSliceUrl) ? base : new URL(String(base));
        if (raw === "") raw = b.href;
        else if (raw.indexOf("//") === 0) raw = b.protocol + raw;
        else if (raw.charCodeAt(0) === 47) raw = b.origin + raw;
        else if (raw.charCodeAt(0) === 63) raw = b.origin + b.pathname + raw;
        else if (raw.charCodeAt(0) === 35) raw = b.origin + b.pathname + b.search + raw;
        else {
          var dir = b.pathname.slice(0, b.pathname.lastIndexOf("/") + 1) || "/";
          raw = b.origin + dir + raw;
        }
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
    if (si <= 0) {
      var ci0 = raw.indexOf(":");
      if (ci0 > 0 && /^[a-zA-Z][a-zA-Z0-9+.-]*$/.test(raw.slice(0, ci0))) {
        var proto0 = raw.slice(0, ci0 + 1).toLowerCase();
        if (proto0 !== "http:" && proto0 !== "https:" && proto0 !== "ws:" && proto0 !== "wss:" && proto0 !== "file:" && proto0 !== "ftp:" && raw.charCodeAt(ci0 + 1) !== 47) {
          this.protocol = proto0;
          this.username = "";
          this.password = "";
          this.hostname = "";
          this.port = "";
          this.host = "";
          this.origin = "null";
          this.pathname = raw.slice(ci0 + 1);
          this.hash = hash === "#" ? "" : hash;
          this.search = search === "?" ? "" : search;
          this.href = this.protocol + this.pathname + this.search + this.hash;
          this._sp = null;
          return;
        }
      }
      throw new TypeError("Invalid URL: " + input);
    }
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
  URL.prototype._isSliceUrl = true;
  Object.defineProperty(URL.prototype, "searchParams", {
    get: function() {
      return this._sp || (this._sp = new URLSearchParams(this.search, this));
    }
  });
  URL.prototype._syncSearch = function() {
    var baseHref = (this.origin === "null" ? this.protocol : this.origin) + this.pathname;
    if (!this._sp || this._sp._ensureP().length === 0) {
      this.search = "";
      this.href = baseHref + this.hash;
      return;
    }
    var qs = this._sp.toString();
    this.search = qs ? "?" + qs : "";
    this.href = baseHref + this.search + this.hash;
  };
  URL.prototype.toString = URL.prototype.toJSON = function() {
    return this.href;
  };

  FastUrlProto = Object.create(URL.prototype);
  function __matFastUrl(u, field, val) {
    var proto = u._rs === 7 ? "http:" : "https:";
    var host = u._r.slice(u._rs, u._pi);
    var pci = host.lastIndexOf(":");
    var hasPort = pci !== -1 && host.indexOf("]") < pci;
    var hostname = hasPort ? host.slice(0, pci) : host;
    var port = hasPort ? host.slice(pci + 1) : "";
    var origin = u._r.slice(0, u._pi);
    var pathname = u._qi === -1 ? u._r.slice(u._pi) : u._r.slice(u._pi, u._qi);
    var search = u._qi === -1 ? "" : u._r.slice(u._qi);
    var href = u._r;
    Object.setPrototypeOf(u, URL.prototype);
    u.protocol = proto;
    u.username = "";
    u.password = "";
    u.hostname = hostname;
    u.port = port;
    u.host = host;
    u.origin = origin;
    u.pathname = pathname;
    u.hash = "";
    u.search = search;
    u.href = href;
    u[field] = val;
  }
  Object.defineProperties(FastUrlProto, {
    href: {
      get: function() { return this._r; },
      set: function(v) { __matFastUrl(this, "href", String(v)); },
      configurable: true,
      enumerable: true
    },
    protocol: {
      get: function() { return this._rs === 7 ? "http:" : "https:"; },
      set: function(v) { __matFastUrl(this, "protocol", String(v)); },
      configurable: true,
      enumerable: true
    },
    username: {
      get: function() { return ""; },
      set: function(v) { __matFastUrl(this, "username", String(v)); },
      configurable: true,
      enumerable: true
    },
    password: {
      get: function() { return ""; },
      set: function(v) { __matFastUrl(this, "password", String(v)); },
      configurable: true,
      enumerable: true
    },
    host: {
      get: function() { return this._r.slice(this._rs, this._pi); },
      set: function(v) { __matFastUrl(this, "host", String(v)); },
      configurable: true,
      enumerable: true
    },
    hostname: {
      get: function() {
        var h = this._r.slice(this._rs, this._pi);
        var pci = h.lastIndexOf(":");
        return (pci !== -1 && h.indexOf("]") < pci) ? h.slice(0, pci) : h;
      },
      set: function(v) { __matFastUrl(this, "hostname", String(v)); },
      configurable: true,
      enumerable: true
    },
    port: {
      get: function() {
        var h = this._r.slice(this._rs, this._pi);
        var pci = h.lastIndexOf(":");
        return (pci !== -1 && h.indexOf("]") < pci) ? h.slice(pci + 1) : "";
      },
      set: function(v) { __matFastUrl(this, "port", String(v)); },
      configurable: true,
      enumerable: true
    },
    origin: {
      get: function() { return this._r.slice(0, this._pi); },
      set: function(v) { __matFastUrl(this, "origin", String(v)); },
      configurable: true,
      enumerable: true
    },
    pathname: {
      get: function() {
        return this._qi === -1 ? this._r.slice(this._pi) : this._r.slice(this._pi, this._qi);
      },
      set: function(v) { __matFastUrl(this, "pathname", String(v)); },
      configurable: true,
      enumerable: true
    },
    search: {
      get: function() {
        return this._qi === -1 ? "" : this._r.slice(this._qi);
      },
      set: function(v) { __matFastUrl(this, "search", String(v)); },
      configurable: true,
      enumerable: true
    },
    hash: {
      get: function() { return ""; },
      set: function(v) { __matFastUrl(this, "hash", String(v)); },
      configurable: true,
      enumerable: true
    },
    searchParams: {
      get: function() {
        return this._sp || (this._sp = new URLSearchParams(this._qi === -1 ? "" : this._r.slice(this._qi + 1), this));
      },
      configurable: true
    }
  });
  FastUrlProto._syncSearch = function() {
    var baseHref = this._qi === -1 ? this._r : this._r.slice(0, this._qi);
    if (!this._sp || this._sp._ensureP().length === 0) {
      this._r = baseHref;
      this._qi = -1;
      return;
    }
    var qs = this._sp.toString();
    if (qs) {
      this._qi = baseHref.length;
      this._r = baseHref + "?" + qs;
    } else {
      this._r = baseHref;
      this._qi = -1;
    }
  };
  URL.canParse = function(url, base) {
    if (arguments.length === 0) return false;
    var u = (url && url._isSliceUrl) ? url.href : url;
    var b = (base && base._isSliceUrl) ? base.href : base;
    return __slice_url_can_parse(u, b);
  };
  var __blobUrlSeq = 1;
  globalThis.__slice_blob_urls = globalThis.__slice_blob_urls || Object.create(null);
  URL.createObjectURL = function(blob) {
    var id = "blob:slice/" + (__blobUrlSeq++);
    var txt = "";
    if (blob && blob._u8 instanceof Uint8Array) {
      __slice_ensure_shard(2);
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

  globalThis.URLSearchParams = URLSearchParams;
  globalThis.URL = URL;
})();
