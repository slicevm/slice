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
  URLSearchParams.prototype._isUSP = true;
  URLSearchParams.prototype._sync = function() {
    if (this._u) this._u._syncSearch();
  };
  URLSearchParams.prototype.get = function(n) {
    var k = typeof n === "string" ? n : String(n), p = this._p;
    for (var i = 0; i < p.length; i++) {
      if (p[i][0] === k) return p[i][1];
    }
    return null;
  };
  URLSearchParams.prototype.getAll = function(n) {
    var k = typeof n === "string" ? n : String(n), p = this._p, r = [];
    for (var i = 0; i < p.length; i++) {
      if (p[i][0] === k) r.push(p[i][1]);
    }
    return r;
  };
  URLSearchParams.prototype.has = function(n) {
    var k = typeof n === "string" ? n : String(n), p = this._p;
    for (var i = 0; i < p.length; i++) {
      if (p[i][0] === k) return true;
    }
    return false;
  };
  URLSearchParams.prototype.set = function(n, v) {
    var k = typeof n === "string" ? n : String(n);
    var s = typeof v === "string" ? v : String(v);
    var p = this._p, len = p.length, fi = -1;
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
    this._p.push([typeof n === "string" ? n : String(n), typeof v === "string" ? v : String(v)]);
    if (this._u) this._u._syncSearch();
  };
  URLSearchParams.prototype.delete = function(n) {
    var k = typeof n === "string" ? n : String(n), p = this._p, nx = [];
    for (var i = 0; i < p.length; i++) {
      if (p[i][0] !== k) nx.push(p[i]);
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
    var p = this._p;
    for (var i = 0; i < p.length; i++) {
      cb.call(ta, p[i][1], p[i][0], this);
    }
  };
  URLSearchParams.prototype.entries = function() {
    return this._p.slice();
  };
  URLSearchParams.prototype.keys = function() {
    var p = this._p, r = [];
    for (var i = 0; i < p.length; i++) r.push(p[i][0]);
    return r;
  };
  URLSearchParams.prototype.values = function() {
    var p = this._p, r = [];
    for (var i = 0; i < p.length; i++) r.push(p[i][1]);
    return r;
  };
  URLSearchParams.prototype.toString = function() {
    var p = this._p, len = p.length;
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
    var raw = (input && input._isSliceUrl) ? input.href : (typeof input === "string" ? input.trim() : String(input).trim());
    if (base !== undefined && raw.indexOf("https://") !== 0 && raw.indexOf("http://") !== 0 && raw.indexOf("://") === -1) {
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
        if (raw.indexOf("//") === 0) raw = b.protocol + raw;
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
  URL.prototype._isSliceUrl = true;
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
