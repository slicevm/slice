// Slice (Nexus-JS) Lazy Prelude Shard 1: WinterTC `URL` and `URLSearchParams` (`01_web_fetch_url.js`)
(function() {
  function __uspD(s) {
    if (s.indexOf("+") === -1 && s.indexOf("%") === -1) return s;
    return __slice_usp_decode(s);
  }

  var __uspEncCache = Object.create(null);
  var __uspEncCount = 0;

  function __uspE(s) {
    var str = typeof s === "string" ? s : String(s), n = str.length;
    if (n === 0) return "";
    if (n <= 48) {
      var hit = __uspEncCache[str];
      if (hit !== undefined) return hit;
    }
    var out;
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
      out = safe ? str : __slice_usp_encode(str);
    } else {
      out = __slice_usp_encode(str);
    }
    if (n <= 48 && __uspEncCount < 512) {
      __uspEncCache[str] = out;
      __uspEncCount++;
    }
    return out;
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
        if (!pr || typeof pr !== "object" || pr.length !== 2) {
          throw new TypeError("URLSearchParams: each sequence item must be a 2-element pair");
        }
        this._p.push([String(pr[0]), String(pr[1])]);
      }
    } else if (typeof init === "object") {
      if (init._isUSP) {
        var sp = init._ensureP();
        for (var m = 0; m < sp.length; m++) this._p.push([sp[m][0], sp[m][1]]);
      } else if (typeof Symbol !== "undefined" && Symbol.iterator && typeof init[Symbol.iterator] === "function") {
        var iter = init[Symbol.iterator](), step;
        while (!(step = iter.next()).done) {
          var item = step.value;
          if (!item || typeof item !== "object") {
            throw new TypeError("URLSearchParams: sequence item must be an object pair");
          }
          var pairArr = Array.isArray(item) ? item : Array.from(item);
          if (pairArr.length !== 2) {
            throw new TypeError("URLSearchParams: sequence item must contain exactly 2 elements");
          }
          this._p.push([String(pairArr[0]), String(pairArr[1])]);
        }
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
    var u = this._u;
    if (u) {
      if (u._isFastUrl) u._sd = true;
      else u._syncSearch();
    }
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
    var k = typeof n === "string" ? n : String(n);
    var p = this._p;
    if (p === null) {
      var q = this._raw;
      if (!q) return [];
      if (q.indexOf("+") === -1 && q.indexOf("%") === -1) {
        var rFast = [], qLen = q.length, kLen = k.length, pos = 0;
        while (pos < qLen) {
          var amp = q.indexOf("&", pos);
          var end = amp === -1 ? qLen : amp;
          var sgLen = end - pos;
          if (sgLen > 0) {
            if (sgLen > kLen && q.charCodeAt(pos + kLen) === 61 && q.startsWith(k, pos)) {
              rFast.push(q.slice(pos + kLen + 1, end));
            } else if (sgLen === kLen && q.startsWith(k, pos)) {
              rFast.push("");
            }
          }
          if (amp === -1) break;
          pos = amp + 1;
        }
        return rFast;
      }
      p = this._ensureP();
    }
    var r = [];
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
    var u = this._u;
    if (u) {
      if (u._isFastUrl) u._sd = true;
      else u._syncSearch();
    }
  };
  URLSearchParams.prototype.append = function(n, v) {
    this._ensureP().push([typeof n === "string" ? n : String(n), typeof v === "string" ? v : String(v)]);
    var u = this._u;
    if (u) {
      if (u._isFastUrl) u._sd = true;
      else u._syncSearch();
    }
  };
  URLSearchParams.prototype.delete = function(n) {
    var k = typeof n === "string" ? n : String(n), p = this._ensureP(), nx = [];
    for (var i = 0; i < p.length; i++) {
      if (p[i][0] !== k) nx.push(p[i]);
    }
    this._p = nx;
    var u = this._u;
    if (u) {
      if (u._isFastUrl) u._sd = true;
      else u._syncSearch();
    }
  };
  URLSearchParams.prototype.sort = function() {
    var p = this._ensureP(), n = p.length;
    if (n > 1) {
      if (n <= 12) {
        for (var i = 1; i < n; i++) {
          var cur = p[i], ck = cur[0], j = i - 1;
          while (j >= 0 && p[j][0] > ck) {
            p[j + 1] = p[j];
            j--;
          }
          p[j + 1] = cur;
        }
      } else {
        p.sort(function(a, b) {
          return a[0] < b[0] ? -1 : (a[0] > b[0] ? 1 : 0);
        });
      }
    }
    var u = this._u;
    if (u) {
      if (u._isFastUrl) u._sd = true;
      else u._syncSearch();
    }
  };
  URLSearchParams.prototype.forEach = function(cb, ta) {
    var p = this._ensureP();
    for (var i = 0; i < p.length; i++) {
      cb.call(ta, p[i][1], p[i][0], this);
    }
  };
  var __uspIterProto = Object.create(Object.getPrototypeOf(Object.getPrototypeOf([][Symbol.iterator]())));
  __uspIterProto.next = function() {
    var p = this._t._ensureP();
    if (this._i >= p.length) return { value: undefined, done: true };
    var pair = p[this._i++];
    var k = this._k;
    return {
      value: k === 0 ? pair[0] : (k === 1 ? pair[1] : [pair[0], pair[1]]),
      done: false
    };
  };
  if (typeof Symbol !== "undefined" && Symbol.toStringTag) {
    Object.defineProperty(__uspIterProto, Symbol.toStringTag, {
      value: "URLSearchParams Iterator",
      configurable: true
    });
  }
  function __mkUspIter(usp, kind) {
    var it = Object.create(__uspIterProto);
    it._t = usp;
    it._i = 0;
    it._k = kind;
    return it;
  }

  URLSearchParams.prototype.entries = function() {
    return __mkUspIter(this, 2);
  };
  URLSearchParams.prototype.keys = function() {
    return __mkUspIter(this, 0);
  };
  URLSearchParams.prototype.values = function() {
    return __mkUspIter(this, 1);
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
    URLSearchParams.prototype[Symbol.iterator] = URLSearchParams.prototype.entries;
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

  function __isSpecialUrlProto(p) {
    return p === "http:" || p === "https:" || p === "ws:" || p === "wss:" || p === "ftp:" || p === "file:";
  }
  function __defaultUrlPort(p) {
    if (p === "http:" || p === "ws:") return "80";
    if (p === "https:" || p === "wss:") return "443";
    if (p === "ftp:") return "21";
    return null;
  }
  function __parseLeadingPort(s) {
    var i = 0, len = s.length;
    while (i < len && s.charCodeAt(i) >= 48 && s.charCodeAt(i) <= 57) i++;
    if (i === 0) return null;
    for (var j = i; j < len; j++) {
      var cj = s.charCodeAt(j);
      if (cj !== 47 && cj !== 92 && cj !== 63 && cj !== 35) return null;
    }
    var n = Number(s.slice(0, i));
    if (!(n >= 0 && n <= 65535)) return null;
    return String(n);
  }
  function __rebuildUrlHref(u) {
    var p = u._proto;
    var defP = __defaultUrlPort(p);
    if (defP !== null && u._port === defP) u._port = "";
    u._host = u._port ? (u._hostname + ":" + u._port) : u._hostname;
    if (p === "http:" || p === "https:" || p === "ws:" || p === "wss:" || p === "ftp:") {
      u._origin = p + "//" + u._host;
      var auth = u._user ? (u._pass ? u._user + ":" + u._pass + "@" : u._user + "@") : "";
      u._href = p + "//" + auth + u._host + u._path + u._search + u._hash;
    } else if (p === "file:") {
      u._origin = "null";
      u._href = "file://" + u._host + u._path + u._search + u._hash;
    } else {
      u._origin = "null";
      u._href = p + (u._host ? "//" + u._host : "") + u._path + u._search + u._hash;
    }
  }
  function __syncExistingSp(u) {
    if (u._sp) {
      u._sp._p = null;
      u._sp._raw = u._search ? u._search.slice(1) : "";
    }
  }
  function __setUrlField(u, field, val) {
    if (field === "href") {
      var parsed = new URL(val);
      if (parsed._isFastUrl) {
        if (parsed._sd) parsed._syncSearch();
        var fh = parsed._r.slice(parsed._rs, parsed._pi);
        var fpci = fh.lastIndexOf(":");
        var fHasPort = fpci !== -1 && fh.indexOf("]") < fpci;
        u._proto = parsed._rs === 7 ? "http:" : "https:";
        u._user = "";
        u._pass = "";
        u._hostname = fHasPort ? fh.slice(0, fpci) : fh;
        u._port = fHasPort ? fh.slice(fpci + 1) : "";
        u._host = fh;
        u._origin = parsed._r.slice(0, parsed._pi);
        u._path = parsed._qi === -1 ? parsed._r.slice(parsed._pi) : parsed._r.slice(parsed._pi, parsed._qi);
        u._hash = "";
        u._search = parsed._qi === -1 ? "" : parsed._r.slice(parsed._qi);
        u._href = parsed._r;
      } else {
        u._proto = parsed._proto;
        u._user = parsed._user;
        u._pass = parsed._pass;
        u._hostname = parsed._hostname;
        u._port = parsed._port;
        u._host = parsed._host;
        u._origin = parsed._origin;
        u._path = parsed._path;
        u._hash = parsed._hash;
        u._search = parsed._search;
        u._href = parsed._href;
      }
      __syncExistingSp(u);
      return;
    }
    if (field === "protocol") {
      var ci = val.indexOf(":");
      var cand = (ci === -1 ? val : val.slice(0, ci)).toLowerCase();
      if (!/^[a-z][a-z0-9+.-]*$/.test(cand)) return;
      var nextP = cand + ":";
      if (__isSpecialUrlProto(u._proto) !== __isSpecialUrlProto(nextP)) return;
      if ((u._user || u._pass || u._port) && nextP === "file:") return;
      if (u._proto === "file:" && !u._hostname && __isSpecialUrlProto(nextP) && nextP !== "file:") return;
      u._proto = nextP;
      __rebuildUrlHref(u);
      return;
    }
    if (field === "username") {
      if (!u._hostname || u._proto === "file:") return;
      u._user = encodeURIComponent(val).replace(/%3A/gi, "%3A");
      __rebuildUrlHref(u);
      return;
    }
    if (field === "password") {
      if (!u._hostname || u._proto === "file:") return;
      u._pass = encodeURIComponent(val);
      __rebuildUrlHref(u);
      return;
    }
    if (field === "host") {
      if (!__isSpecialUrlProto(u._proto) && !u._hostname) return;
      var cutH = val.search(/[\/\\?#]/);
      var hp = cutH === -1 ? val : val.slice(0, cutH);
      if (!hp) return;
      var hpci = hp.lastIndexOf(":");
      var hasCol = hpci !== -1 && hp.indexOf("]") < hpci;
      var nextHn = (hasCol ? hp.slice(0, hpci) : hp).toLowerCase();
      if (!nextHn) return;
      if (hasCol) {
        var prtStr = hp.slice(hpci + 1);
        if (prtStr !== "") {
          var parsedP = __parseLeadingPort(prtStr);
          if (parsedP === null) return;
          u._port = parsedP;
        }
      }
      u._hostname = nextHn;
      __rebuildUrlHref(u);
      return;
    }
    if (field === "hostname") {
      if (!__isSpecialUrlProto(u._proto) && !u._hostname) return;
      var cutHn = val.search(/[\/\\?#]/);
      var rawHn = cutHn === -1 ? val : val.slice(0, cutHn);
      var hCol = rawHn.indexOf(":");
      if (hCol !== -1 && !(rawHn.charCodeAt(0) === 91 && rawHn.charCodeAt(rawHn.length - 1) === 93)) {
        rawHn = rawHn.slice(0, hCol);
      }
      var lowHn = rawHn.toLowerCase();
      if (!lowHn && u._proto !== "file:") return;
      u._hostname = lowHn;
      __rebuildUrlHref(u);
      return;
    }
    if (field === "port") {
      if (!u._hostname || u._proto === "file:") return;
      if (val === "") {
        u._port = "";
        __rebuildUrlHref(u);
        return;
      }
      var nextPrt = __parseLeadingPort(val);
      if (nextPrt === null) return;
      u._port = nextPrt;
      __rebuildUrlHref(u);
      return;
    }
    if (field === "pathname") {
      if (!__isSpecialUrlProto(u._proto) && !u._hostname) return;
      var pNorm = val.replace(/\\/g, "/");
      u._path = pNorm && pNorm.charCodeAt(0) === 47 ? __normP(pNorm) : __normP("/" + pNorm);
      __rebuildUrlHref(u);
      return;
    }
    if (field === "search") {
      u._search = !val || val === "?" ? "" : (val.charCodeAt(0) === 63 ? val : "?" + val);
      __syncExistingSp(u);
      __rebuildUrlHref(u);
      return;
    }
    if (field === "hash") {
      u._hash = !val || val === "#" ? "" : (val.charCodeAt(0) === 35 ? val : "#" + val);
      __rebuildUrlHref(u);
    }
  }

  function __hasRfc3986Scheme(s) {
    var len = s.length;
    if (len < 2) return false;
    var c0 = s.charCodeAt(0);
    if (!((c0 >= 65 && c0 <= 90) || (c0 >= 97 && c0 <= 122))) return false;
    for (var i = 1; i < len; i++) {
      var c = s.charCodeAt(i);
      if (c === 58) return true;
      var ok =
        (c >= 65 && c <= 90) ||
        (c >= 97 && c <= 122) ||
        (c >= 48 && c <= 57) ||
        c === 43 ||
        c === 45 ||
        c === 46;
      if (!ok) return false;
    }
    return false;
  }

  var FastUrlProto;
  var __originCache = Object.create(null);
  var __originCacheCount = 0;
  var __fastUrlSpecCache = Object.create(null);
  var __fastUrlSpecCount = 0;

  function __FastUrl(s, rs, pi, qi) {
    this._r = s;
    this._rs = rs;
    this._pi = pi;
    this._qi = qi;
    this._sp = null;
    this._sd = false;
  }

  function __fastHttpOrigin(b) {
    var len = b.length;
    if (len < 8) return "";
    if (len <= 96) {
      var hit = __originCache[b];
      if (hit !== undefined) return hit;
    }
    var end = __fastHttpOriginEnd(b);
    var res = end === -1 ? "" : (end === len ? b : b.slice(0, end));
    if (len <= 96 && __originCacheCount < 256) {
      __originCache[b] = res;
      __originCacheCount++;
    }
    return res;
  }

  function __fastHttpOriginEnd(b) {
    var len = b.length;
    if (len < 8 || b.charCodeAt(0) !== 104 || b.charCodeAt(len - 1) <= 32) return -1;
    var rs = 0;
    if (b.charCodeAt(1) === 116 && b.charCodeAt(2) === 116 && b.charCodeAt(3) === 112) {
      if (b.charCodeAt(4) === 58 && b.charCodeAt(5) === 47 && b.charCodeAt(6) === 47) rs = 7;
      else if (b.charCodeAt(4) === 115 && b.charCodeAt(5) === 58 && b.charCodeAt(6) === 47 && b.charCodeAt(7) === 47) rs = 8;
    }
    if (rs === 0) return -1;
    var end = len;
    for (var i = rs; i < len; i++) {
      var c = b.charCodeAt(i);
      if (c <= 32) return -1;
      if (c === 47 || c === 63 || c === 35) {
        end = i;
        break;
      }
    }
    if (end <= rs) return -1;
    var colon = -1;
    for (var j = rs; j < end; j++) {
      var hc = b.charCodeAt(j);
      if (hc === 58) {
        if (colon !== -1) return -1;
        colon = j;
      } else if (!((hc >= 97 && hc <= 122) || (hc >= 48 && hc <= 57) || hc === 46 || hc === 45)) {
        return -1;
      }
    }
    if (colon !== -1) {
      if (colon === rs || colon + 1 >= end || end - colon - 1 > 5) return -1;
      var portNum = 0;
      for (var p = colon + 1; p < end; p++) {
        var dc = b.charCodeAt(p);
        if (dc < 48 || dc > 57) return -1;
        portNum = portNum * 10 + (dc - 48);
      }
      if (portNum > 65535 || (rs === 7 && portNum === 80) || (rs === 8 && portNum === 443)) return -1;
    }
    for (var r = end + 1; r < len; r++) {
      if (b.charCodeAt(r) <= 32) return -1;
    }
    return end;
  }

  function __tryFastHttpUrl(s) {
    var origS = s;
    var origLen = origS.length;
    if (origLen < 8) return null;
    if (origLen <= 128) {
      var cached = __fastUrlSpecCache[origS];
      if (cached !== undefined) {
        return cached === null ? null : new __FastUrl(cached[0], cached[1], cached[2], cached[3]);
      }
    }
    var len = origLen;
    if (s.charCodeAt(0) !== 104 || s.charCodeAt(len - 1) <= 32) return null;
    var rs = 0;
    if (s.charCodeAt(1) === 116 && s.charCodeAt(2) === 116 && s.charCodeAt(3) === 112) {
      if (s.charCodeAt(4) === 58 && s.charCodeAt(5) === 47 && s.charCodeAt(6) === 47) rs = 7;
      else if (s.charCodeAt(4) === 115 && s.charCodeAt(5) === 58 && s.charCodeAt(6) === 47 && s.charCodeAt(7) === 47) rs = 8;
    }
    if (rs === 0 || s.indexOf("#", rs) !== -1) return null;
    var qi = s.indexOf("?", rs);
    var pathEnd = qi === -1 ? len : qi;
    var pi = s.indexOf("/", rs);
    if (pi === -1 && qi === -1 && len > rs) {
      s = s + "/";
      len += 1;
      pi = len - 1;
      pathEnd = len;
    } else if (pi <= rs || pi > pathEnd) {
      return null;
    }
    var colonIdx = -1;
    for (var i = rs; i < pi; i++) {
      var c = s.charCodeAt(i);
      if (c === 58) {
        if (colonIdx !== -1) return null;
        colonIdx = i;
      } else if (!((c >= 97 && c <= 122) || (c >= 48 && c <= 57) || c === 46 || c === 45)) {
        return null;
      }
    }
    if (colonIdx !== -1) {
      if (colonIdx === rs || colonIdx + 1 >= pi || pi - colonIdx - 1 > 5) return null;
      var pVal = 0;
      for (var pk = colonIdx + 1; pk < pi; pk++) {
        var pd = s.charCodeAt(pk);
        if (pd < 48 || pd > 57) return null;
        pVal = pVal * 10 + (pd - 48);
      }
      if (pVal > 65535 || (rs === 7 && pVal === 80) || (rs === 8 && pVal === 443)) return null;
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
    if (origLen <= 128 && __fastUrlSpecCount < 256) {
      __fastUrlSpecCache[origS] = [s, rs, pi, qi];
      __fastUrlSpecCount++;
    }
    return new __FastUrl(s, rs, pi, qi);
  }

  function URL(input, base) {
    if (base === undefined) {
      if (typeof input === "string") {
        var fastU = __tryFastHttpUrl(input);
        if (fastU !== null) return fastU;
      }
    } else if (typeof input === "string" && input.length > 0) {
      if (typeof base === "string") {
        var bOrig = __fastHttpOrigin(base);
        if (bOrig !== "") {
          var ic0 = input.charCodeAt(0);
          if (ic0 === 104) {
            var fastAbs = __tryFastHttpUrl(input);
            if (fastAbs !== null) return fastAbs;
          } else if (ic0 === 47 && (input.length === 1 || input.charCodeAt(1) !== 47)) {
            var fastRel = __tryFastHttpUrl(bOrig + input);
            if (fastRel !== null) return fastRel;
          }
        }
      } else if (base && base._isSliceUrl) {
        var ic1 = input.charCodeAt(0);
        if (ic1 === 104) {
          var fastAbsU = __tryFastHttpUrl(input);
          if (fastAbsU !== null) return fastAbsU;
        } else if (ic1 === 47 && (input.length === 1 || input.charCodeAt(1) !== 47)) {
          var uOrig = base.origin;
          if (uOrig !== "null") {
            var fastRelU = __tryFastHttpUrl(uOrig + input);
            if (fastRelU !== null) return fastRelU;
          }
        }
      }
    }
    var raw = (input && input._isSliceUrl) ? input.href : (typeof input === "string" ? input.trim() : String(input).trim());
    if (base !== undefined) {
      var b = (base && base._isSliceUrl) ? base : new URL(typeof base === "string" ? base : String(base));
      if (!__hasRfc3986Scheme(raw)) {
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
          this._proto = proto0;
          this._user = "";
          this._pass = "";
          this._hostname = "";
          this._port = "";
          this._host = "";
          this._origin = "null";
          this._path = raw.slice(ci0 + 1);
          this._hash = hash === "#" ? "" : hash;
          this._search = search === "?" ? "" : search;
          this._href = this._proto + this._path + this._search + this._hash;
          this._sp = null;
          return;
        }
      }
      throw new TypeError("Invalid URL: " + input);
    }
    this._proto = raw.slice(0, si + 1).toLowerCase();
    var rest = raw.slice(si + 3);
    var pi = rest.indexOf("/");
    var authHost = pi === -1 ? rest : rest.slice(0, pi);
    var pathRaw = pi === -1 ? "/" : rest.slice(pi);
    var ai = authHost.indexOf("@"), hostPort = authHost;
    this._user = "";
    this._pass = "";
    if (ai !== -1) {
      var up = authHost.slice(0, ai), ci = up.indexOf(":");
      this._user = ci === -1 ? up : up.slice(0, ci);
      this._pass = ci === -1 ? "" : up.slice(ci + 1);
      hostPort = authHost.slice(ai + 1);
    }
    if (
      !hostPort &&
      (this._proto === "http:" ||
        this._proto === "https:" ||
        this._proto === "ws:" ||
        this._proto === "wss:" ||
        this._proto === "ftp:")
    ) {
      throw new TypeError("Invalid URL: " + input);
    }
    var pci = hostPort.lastIndexOf(":");
    if (pci !== -1 && hostPort.indexOf("]") < pci) {
      this._hostname = hostPort.slice(0, pci).toLowerCase();
      this._port = hostPort.slice(pci + 1);
    } else {
      this._hostname = hostPort.toLowerCase();
      this._port = "";
    }
    this._path = __normP(pathRaw);
    this._hash = hash === "#" ? "" : hash;
    this._search = search === "?" ? "" : search;
    this._sp = null;
    __rebuildUrlHref(this);
  }
  URL.prototype._isSliceUrl = true;
  Object.defineProperties(URL.prototype, {
    href: {
      get: function() { return this._href; },
      set: function(v) { __setUrlField(this, "href", String(v)); },
      configurable: true,
      enumerable: true
    },
    origin: {
      get: function() { return this._origin; },
      configurable: true,
      enumerable: true
    },
    protocol: {
      get: function() { return this._proto; },
      set: function(v) { __setUrlField(this, "protocol", String(v)); },
      configurable: true,
      enumerable: true
    },
    username: {
      get: function() { return this._user; },
      set: function(v) { __setUrlField(this, "username", String(v)); },
      configurable: true,
      enumerable: true
    },
    password: {
      get: function() { return this._pass; },
      set: function(v) { __setUrlField(this, "password", String(v)); },
      configurable: true,
      enumerable: true
    },
    host: {
      get: function() { return this._host; },
      set: function(v) { __setUrlField(this, "host", String(v)); },
      configurable: true,
      enumerable: true
    },
    hostname: {
      get: function() { return this._hostname; },
      set: function(v) { __setUrlField(this, "hostname", String(v)); },
      configurable: true,
      enumerable: true
    },
    port: {
      get: function() { return this._port; },
      set: function(v) { __setUrlField(this, "port", String(v)); },
      configurable: true,
      enumerable: true
    },
    pathname: {
      get: function() { return this._path; },
      set: function(v) { __setUrlField(this, "pathname", String(v)); },
      configurable: true,
      enumerable: true
    },
    search: {
      get: function() { return this._search; },
      set: function(v) { __setUrlField(this, "search", String(v)); },
      configurable: true,
      enumerable: true
    },
    hash: {
      get: function() { return this._hash; },
      set: function(v) { __setUrlField(this, "hash", String(v)); },
      configurable: true,
      enumerable: true
    },
    searchParams: {
      get: function() {
        return this._sp || (this._sp = new URLSearchParams(this._search, this));
      },
      configurable: true,
      enumerable: true
    }
  });
  URL.prototype._syncSearch = function() {
    if (!this._sp || this._sp._ensureP().length === 0) {
      this._search = "";
    } else {
      var qs = this._sp.toString();
      this._search = qs ? "?" + qs : "";
    }
    __rebuildUrlHref(this);
  };
  URL.prototype.toString = URL.prototype.toJSON = function() {
    return this.href;
  };

  FastUrlProto = Object.create(URL.prototype);
  FastUrlProto._isSliceUrl = true;
  FastUrlProto._isFastUrl = true;
  FastUrlProto.toString = FastUrlProto.toJSON = URL.prototype.toString;
  __FastUrl.prototype = FastUrlProto;
  function __matFastUrl(u, field, val) {
    if (u._sd) u._syncSearch();
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
    u._isFastUrl = false;
    Object.setPrototypeOf(u, URL.prototype);
    u._proto = proto;
    u._user = "";
    u._pass = "";
    u._hostname = hostname;
    u._port = port;
    u._host = host;
    u._origin = origin;
    u._path = pathname;
    u._hash = "";
    u._search = search;
    u._href = href;
    __setUrlField(u, field, val);
  }
  Object.defineProperties(FastUrlProto, {
    href: {
      get: function() {
        if (this._sd) this._syncSearch();
        return this._r;
      },
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
        if (this._sd) this._syncSearch();
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
    this._sd = false;
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
