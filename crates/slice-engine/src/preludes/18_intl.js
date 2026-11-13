// Slice (Nexus-JS) Prelude Shard 18: WinterTC `Intl` (`18_intl.js`)
(function() {
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

  var __TZ_TABLE = {
    "utc": [0, 0, 0, "UTC"],
    "etc/utc": [0, 0, 0, "Etc/UTC"],
    "etc/gmt": [0, 0, 0, "Etc/GMT"],
    "gmt": [0, 0, 0, "GMT"],
    "z": [0, 0, 0, "UTC"],
    "asia/tokyo": [540, 540, 0, "Asia/Tokyo"],
    "asia/seoul": [540, 540, 0, "Asia/Seoul"],
    "asia/shanghai": [480, 480, 0, "Asia/Shanghai"],
    "asia/hong_kong": [480, 480, 0, "Asia/Hong_Kong"],
    "asia/singapore": [480, 480, 0, "Asia/Singapore"],
    "asia/taipei": [480, 480, 0, "Asia/Taipei"],
    "asia/manila": [480, 480, 0, "Asia/Manila"],
    "australia/perth": [480, 480, 0, "Australia/Perth"],
    "asia/kolkata": [330, 330, 0, "Asia/Kolkata"],
    "asia/calcutta": [330, 330, 0, "Asia/Kolkata"],
    "asia/dubai": [240, 240, 0, "Asia/Dubai"],
    "asia/bangkok": [420, 420, 0, "Asia/Bangkok"],
    "asia/jakarta": [420, 420, 0, "Asia/Jakarta"],
    "asia/ho_chi_minh": [420, 420, 0, "Asia/Ho_Chi_Minh"],
    "asia/jerusalem": [120, 180, 2, "Asia/Jerusalem"],
    "asia/tel_aviv": [120, 180, 2, "Asia/Tel_Aviv"],
    "europe/london": [0, 60, 2, "Europe/London"],
    "europe/dublin": [0, 60, 2, "Europe/Dublin"],
    "europe/lisbon": [0, 60, 2, "Europe/Lisbon"],
    "europe/berlin": [60, 120, 2, "Europe/Berlin"],
    "europe/paris": [60, 120, 2, "Europe/Paris"],
    "europe/rome": [60, 120, 2, "Europe/Rome"],
    "europe/madrid": [60, 120, 2, "Europe/Madrid"],
    "europe/amsterdam": [60, 120, 2, "Europe/Amsterdam"],
    "europe/brussels": [60, 120, 2, "Europe/Brussels"],
    "europe/vienna": [60, 120, 2, "Europe/Vienna"],
    "europe/warsaw": [60, 120, 2, "Europe/Warsaw"],
    "europe/zurich": [60, 120, 2, "Europe/Zurich"],
    "europe/stockholm": [60, 120, 2, "Europe/Stockholm"],
    "europe/prague": [60, 120, 2, "Europe/Prague"],
    "europe/budapest": [60, 120, 2, "Europe/Budapest"],
    "europe/copenhagen": [60, 120, 2, "Europe/Copenhagen"],
    "europe/oslo": [60, 120, 2, "Europe/Oslo"],
    "europe/athens": [120, 180, 2, "Europe/Athens"],
    "europe/helsinki": [120, 180, 2, "Europe/Helsinki"],
    "europe/bucharest": [120, 180, 2, "Europe/Bucharest"],
    "europe/kiev": [120, 180, 2, "Europe/Kyiv"],
    "europe/kyiv": [120, 180, 2, "Europe/Kyiv"],
    "africa/cairo": [120, 180, 2, "Africa/Cairo"],
    "europe/moscow": [180, 180, 0, "Europe/Moscow"],
    "europe/istanbul": [180, 180, 0, "Europe/Istanbul"],
    "asia/riyadh": [180, 180, 0, "Asia/Riyadh"],
    "africa/nairobi": [180, 180, 0, "Africa/Nairobi"],
    "america/new_york": [-300, -240, 1, "America/New_York"],
    "america/toronto": [-300, -240, 1, "America/Toronto"],
    "america/detroit": [-300, -240, 1, "America/Detroit"],
    "america/montreal": [-300, -240, 1, "America/Montreal"],
    "us/eastern": [-300, -240, 1, "America/New_York"],
    "america/chicago": [-360, -300, 1, "America/Chicago"],
    "america/winnipeg": [-360, -300, 1, "America/Winnipeg"],
    "us/central": [-360, -300, 1, "America/Chicago"],
    "america/denver": [-420, -360, 1, "America/Denver"],
    "america/edmonton": [-420, -360, 1, "America/Edmonton"],
    "us/mountain": [-420, -360, 1, "America/Denver"],
    "america/phoenix": [-420, -420, 0, "America/Phoenix"],
    "america/los_angeles": [-480, -420, 1, "America/Los_Angeles"],
    "america/vancouver": [-480, -420, 1, "America/Vancouver"],
    "us/pacific": [-480, -420, 1, "America/Los_Angeles"],
    "america/anchorage": [-540, -480, 1, "America/Anchorage"],
    "pacific/honolulu": [-600, -600, 0, "Pacific/Honolulu"],
    "us/hawaii": [-600, -600, 0, "Pacific/Honolulu"],
    "america/sao_paulo": [-180, -180, 0, "America/Sao_Paulo"],
    "america/argentina/buenos_aires": [-180, -180, 0, "America/Argentina/Buenos_Aires"],
    "america/buenos_aires": [-180, -180, 0, "America/Argentina/Buenos_Aires"],
    "america/mexico_city": [-360, -360, 0, "America/Mexico_City"],
    "america/bogota": [-300, -300, 0, "America/Bogota"],
    "america/lima": [-300, -300, 0, "America/Lima"],
    "australia/sydney": [600, 600, 0, "Australia/Sydney"],
    "australia/melbourne": [600, 600, 0, "Australia/Melbourne"],
    "pacific/auckland": [720, 720, 0, "Pacific/Auckland"]
  };

  function __resolveTz(tzInput) {
    if (tzInput === undefined) return [0, 0, 0, "UTC"];
    var raw = String(tzInput).trim();
    var entry = __TZ_TABLE[raw.toLowerCase()];
    if (entry) return entry;
    var m = raw.match(/^([+-])(\d{2})(?::?(\d{2}))?$/);
    if (m) {
      var sign = m[1] === "-" ? -1 : 1;
      var hh = Number(m[2]), mm = Number(m[3] || 0);
      if (hh <= 23 && mm <= 59) {
        var mins = sign * (hh * 60 + mm);
        var canon = (sign < 0 ? "-" : "+") + __pad2(hh) + ":" + __pad2(mm);
        return [mins, mins, 0, canon];
      }
    }
    throw new RangeError("Invalid time zone specified: " + raw);
  }

  function __tzOffsetMinutes(tzEntry, utcMs) {
    var std = tzEntry[0], dst = tzEntry[1], rule = tzEntry[2];
    if (rule === 0 || std === dst) return std;
    var d = new Date(utcMs);
    var m = d.getUTCMonth();
    if (rule === 1) {
      if (m > 2 && m < 10) return dst;
      if (m < 2 || m > 10) return std;
      var yr = d.getUTCFullYear();
      if (m === 2) {
        var mar1Day = new Date(Date.UTC(yr, 2, 1)).getUTCDay();
        var secondSun = 8 + ((7 - mar1Day) % 7);
        var startUtc = Date.UTC(yr, 2, secondSun, 2, 0, 0) - std * 60000;
        return utcMs >= startUtc ? dst : std;
      }
      var nov1Day = new Date(Date.UTC(yr, 10, 1)).getUTCDay();
      var firstSun = 1 + ((7 - nov1Day) % 7);
      var endUtc = Date.UTC(yr, 10, firstSun, 2, 0, 0) - dst * 60000;
      return utcMs < endUtc ? dst : std;
    }
    if (rule === 2) {
      if (m > 2 && m < 9) return dst;
      if (m < 2 || m > 9) return std;
      var yr2 = d.getUTCFullYear();
      if (m === 2) {
        var mar31Day = new Date(Date.UTC(yr2, 2, 31)).getUTCDay();
        var lastSunMar = 31 - mar31Day;
        return utcMs >= Date.UTC(yr2, 2, lastSunMar, 1, 0, 0) ? dst : std;
      }
      var oct31Day = new Date(Date.UTC(yr2, 9, 31)).getUTCDay();
      var lastSunOct = 31 - oct31Day;
      return utcMs < Date.UTC(yr2, 9, lastSunOct, 1, 0, 0) ? dst : std;
    }
    return std;
  }

  function __defaultHour12(locale) {
    var l = String(locale || "en-US");
    if (l === "en" || l === "en-US" || l === "en-CA" || l === "en-AU" || l === "en-NZ" || l === "en-IN" || l === "en-PH") {
      return true;
    }
    return false;
  }

  function DateTimeFormat(locales, options) {
    if (!(this instanceof DateTimeFormat)) return new DateTimeFormat(locales, options);
    var o = options || {};
    this._locale = __normLocale(locales);
    this._tzEntry = __resolveTz(o.timeZone);
    this._timeZone = this._tzEntry[3];
    this._dateStyle = o.dateStyle;
    this._timeStyle = o.timeStyle;
    var hasCustom = o.year || o.month || o.day || o.weekday || o.hour || o.minute || o.second || o.dateStyle || o.timeStyle;
    var hasDatePreset = !hasCustom || Boolean(o.dateStyle);
    this._year = o.year || (hasDatePreset ? (o.dateStyle === "short" ? "2-digit" : "numeric") : undefined);
    this._month = o.month || (!hasCustom ? "numeric" : (o.dateStyle === "full" || o.dateStyle === "long" ? "long" : (o.dateStyle === "medium" ? "short" : (o.dateStyle === "short" ? "numeric" : undefined))));
    this._day = o.day || (hasDatePreset ? "numeric" : undefined);
    this._weekday = o.weekday || (o.dateStyle === "full" ? "long" : undefined);
    this._hour = o.hour || (o.timeStyle ? "numeric" : undefined);
    this._minute = o.minute || (o.timeStyle ? "2-digit" : undefined);
    this._second = o.second || (o.timeStyle === "full" || o.timeStyle === "long" || o.timeStyle === "medium" ? "2-digit" : undefined);
    this._hour12 = o.hour12 !== undefined ? Boolean(o.hour12) : (o.hourCycle ? (o.hourCycle === "h11" || o.hourCycle === "h12") : __defaultHour12(this._locale));
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
    var rawD = date === undefined ? new Date(Date.now()) : (date instanceof Date ? date : new Date(Number(date)));
    var utcMs = rawD.getTime();
    if (Number.isNaN(utcMs)) throw new RangeError("Invalid time value");
    var offMin = __tzOffsetMinutes(this._tzEntry, utcMs);
    var d = new Date(utcMs + offMin * 60000);
    var parts = [];
    if (this._weekday) {
      var wd = d.getUTCDay();
      var wVal = this._weekday === "long" ? __DAYS_LONG[wd] : (this._weekday === "narrow" ? __DAYS_SHORT[wd].charAt(0) : __DAYS_SHORT[wd]);
      parts.push({ type: "weekday", value: wVal });
      if (this._month || this._day || this._year) parts.push({ type: "literal", value: ", " });
    }
    var hasDate = Boolean(this._year || this._month || this._day);
    var lang = this._locale.split("-")[0];
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
        var isoOrder = lang === "sv" || lang === "ja" || lang === "zh" || lang === "ko" || lang === "iso";
        var dotDmy = lang === "de" || lang === "ru" || lang === "pl" || lang === "uk" || lang === "cs" || lang === "fi" || lang === "no" || lang === "nb" || lang === "tr";
        var slashDmy = lang === "fr" || lang === "es" || lang === "it" || lang === "pt" || lang === "nl" || this._locale === "en-GB" || this._locale === "en-AU";
        if (isoOrder) {
          if (this._year) parts.push({ type: "year", value: yrStr });
          if (this._year && this._month) parts.push({ type: "literal", value: "-" });
          if (this._month) parts.push({ type: "month", value: __pad2(mIdx + 1) });
          if (this._month && this._day) parts.push({ type: "literal", value: "-" });
          if (this._day) parts.push({ type: "day", value: __pad2(dayNum) });
        } else if (dotDmy) {
          if (this._day) parts.push({ type: "day", value: dayStr });
          if (this._day && this._month) parts.push({ type: "literal", value: "." });
          if (this._month) parts.push({ type: "month", value: mNumStr });
          if ((this._day || this._month) && this._year) parts.push({ type: "literal", value: "." });
          if (this._year) parts.push({ type: "year", value: yrStr });
        } else if (slashDmy) {
          if (this._day) parts.push({ type: "day", value: this._day === "2-digit" ? dayStr : __pad2(dayNum) });
          if (this._day && this._month) parts.push({ type: "literal", value: "/" });
          if (this._month) parts.push({ type: "month", value: this._month === "2-digit" ? mNumStr : __pad2(mIdx + 1) });
          if ((this._day || this._month) && this._year) parts.push({ type: "literal", value: "/" });
          if (this._year) parts.push({ type: "year", value: yrStr });
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
        var padHr = this._hour === "2-digit" || (!this._hour12 && Boolean(this._minute));
        parts.push({ type: "hour", value: padHr ? __pad2(dispHr) : String(dispHr) });
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

  var __CURR_SYM = { USD: "$", EUR: "€", GBP: "£", JPY: "¥", ILS: "₪", CAD: "CA$", AUD: "A$", CHF: "CHF", CNY: "¥", INR: "₹", BRL: "R$", KRW: "₩" };

  function __numLocaleFormat(locale) {
    var lang = String(locale || "en-US").split("-")[0];
    if (lang === "de" || lang === "es" || lang === "it" || lang === "nl" || lang === "pt" || lang === "tr" || lang === "vi" || lang === "id" || lang === "da" || lang === "el" || lang === "ro" || lang === "hu") {
      return { grp: ".", dec: ",", currSuffix: locale !== "pt-BR" && lang !== "nl" };
    }
    if (lang === "fr" || lang === "ru" || lang === "pl" || lang === "uk" || lang === "sv" || lang === "fi" || lang === "cs" || lang === "nb" || lang === "no" || lang === "sk") {
      return { grp: "\u202f", dec: ",", currSuffix: true };
    }
    return { grp: ",", dec: ".", currSuffix: false };
  }

  function NumberFormat(locales, options) {
    if (!(this instanceof NumberFormat)) return new NumberFormat(locales, options);
    var o = options || {};
    this._locale = __normLocale(locales);
    this._style = o.style || "decimal";
    this._currency = o.currency ? String(o.currency).toUpperCase() : undefined;
    if (this._style === "currency" && !this._currency) {
      throw new TypeError("Currency code is required with currency style");
    }
    this._currencyDisplay = o.currencyDisplay || "symbol";
    this._unit = o.unit;
    this._unitDisplay = o.unitDisplay || "short";
    this._notation = o.notation || "standard";
    this._signDisplay = o.signDisplay || "auto";
    this._useGrouping = o.useGrouping !== undefined ? Boolean(o.useGrouping) : true;
    this._minInt = o.minimumIntegerDigits !== undefined ? Number(o.minimumIntegerDigits) : 1;
    var defMinFrac = this._style === "currency" ? (this._currency === "JPY" || this._currency === "KRW" ? 0 : 2) : 0;
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
    var locFmt = __numLocaleFormat(this._locale);
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
    if (fixed.indexOf("e") !== -1 || fixed.indexOf("E") !== -1) {
      var mExp = fixed.match(/^(\d+)(?:\.(\d+))?e([+-]\d+)$/i);
      if (mExp) {
        var expVal = Number(mExp[3]);
        if (expVal >= 0) {
          var mant = mExp[1] + (mExp[2] || "");
          var zCount = expVal - (mExp[2] ? mExp[2].length : 0);
          fixed = zCount >= 0 ? (mant + "0".repeat(zCount)) : (mant.slice(0, mant.length + zCount) + "." + mant.slice(mant.length + zCount));
        }
      }
    }
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
    var currSym = "";
    if (this._style === "currency" && this._currency) {
      currSym = this._currencyDisplay === "code"
        ? this._currency
        : (__CURR_SYM[this._currency] || this._currency);
      if (!locFmt.currSuffix) {
        parts.push({ type: "currency", value: this._currencyDisplay === "code" ? (currSym + "\u00a0") : currSym });
      }
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
        if (i > 0) parts.push({ type: "group", value: locFmt.grp });
        parts.push({ type: "integer", value: groups[i] });
      }
    } else {
      parts.push({ type: "integer", value: intStr });
    }
    if (fracStr.length > 0) {
      parts.push({ type: "decimal", value: locFmt.dec });
      parts.push({ type: "fraction", value: fracStr });
    }
    if (compactSuffix) {
      parts.push({ type: "compact", value: compactSuffix });
    }
    if (this._style === "currency" && this._currency && locFmt.currSuffix) {
      parts.push({ type: "literal", value: "\u00a0" });
      parts.push({ type: "currency", value: currSym });
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

  var __DIACRITIC_BASE = {
    "à": "a", "á": "a", "â": "a", "ã": "a", "ä": "a", "å": "a", "ā": "a", "ă": "a", "ą": "a", "æ": "ae",
    "ç": "c", "ć": "c", "č": "c", "ĉ": "c", "ď": "d", "đ": "d",
    "è": "e", "é": "e", "ê": "e", "ë": "e", "ē": "e", "ĕ": "e", "ė": "e", "ę": "e", "ě": "e",
    "ğ": "g", "ĝ": "g", "ĥ": "h",
    "ì": "i", "í": "i", "î": "i", "ï": "i", "ī": "i", "ĭ": "i", "į": "i", "ı": "i",
    "ĵ": "j", "ķ": "k", "ĺ": "l", "ļ": "l", "ľ": "l", "ł": "l",
    "ñ": "n", "ń": "n", "ņ": "n", "ň": "n",
    "ò": "o", "ó": "o", "ô": "o", "õ": "o", "ö": "o", "ø": "o", "ō": "o", "ŏ": "o", "ő": "o", "œ": "oe",
    "ŕ": "r", "ř": "r", "ś": "s", "ŝ": "s", "ş": "s", "š": "s", "ß": "ss",
    "ť": "t", "ţ": "t",
    "ù": "u", "ú": "u", "û": "u", "ü": "u", "ū": "u", "ŭ": "u", "ů": "u", "ű": "u", "ų": "u",
    "ŵ": "w", "ý": "y", "ÿ": "y", "ŷ": "y", "ź": "z", "ż": "z", "ž": "z"
  };

  function __colBaseKey(s) {
    var low = s.toLowerCase();
    var out = "";
    for (var i = 0; i < low.length; i++) {
      var ch = low.charAt(i);
      var code = ch.charCodeAt(0);
      if (code >= 0x0300 && code <= 0x036f) continue;
      out += __DIACRITIC_BASE[ch] !== undefined ? __DIACRITIC_BASE[ch] : ch;
    }
    return out;
  }

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
          var c = this._cmpChunk(pa, pb);
          if (c !== 0) return c;
        }
      }
      return ta.length === tb.length ? 0 : (ta.length < tb.length ? -1 : 1);
    }
    return this._cmpChunk(a, b);
  };
  Collator.prototype._cmpChunk = function(a, b) {
    if (a === b) return 0;
    var baseA = __colBaseKey(a), baseB = __colBaseKey(b);
    if (baseA !== baseB) return baseA < baseB ? -1 : 1;
    if (this._sensitivity === "base") return 0;
    var lowA = a.toLowerCase(), lowB = b.toLowerCase();
    if (this._sensitivity !== "case" && lowA !== lowB) {
      return lowA < lowB ? -1 : 1;
    }
    if (this._sensitivity === "accent") return 0;
    if (a === b) return 0;
    if (this._caseFirst === "upper") {
      return a < b ? -1 : 1;
    }
    return a > b ? -1 : 1;
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
    var num = Number(n);
    if (!Number.isFinite(num)) return "other";
    var v = Math.abs(num);
    var i = Math.floor(v);
    var isInt = v === i;
    var mod10 = i % 10, mod100 = i % 100;
    var lang = this._locale.split("-")[0];
    if (this._type === "ordinal") {
      if (lang === "en") {
        if (mod10 === 1 && mod100 !== 11) return "one";
        if (mod10 === 2 && mod100 !== 12) return "two";
        if (mod10 === 3 && mod100 !== 13) return "few";
      } else if (lang === "fr" && v === 1) {
        return "one";
      }
      return "other";
    }
    if (lang === "ar") {
      if (v === 0) return "zero";
      if (v === 1) return "one";
      if (v === 2) return "two";
      if (isInt && mod100 >= 3 && mod100 <= 10) return "few";
      if (isInt && mod100 >= 11 && mod100 <= 99) return "many";
      return "other";
    }
    if (lang === "ru" || lang === "uk") {
      if (isInt && mod10 === 1 && mod100 !== 11) return "one";
      if (isInt && mod10 >= 2 && mod10 <= 4 && (mod100 < 12 || mod100 > 14)) return "few";
      if (isInt && (mod10 === 0 || (mod10 >= 5 && mod10 <= 9) || (mod100 >= 11 && mod100 <= 14))) return "many";
      return "other";
    }
    if (lang === "pl") {
      if (isInt && i === 1) return "one";
      if (isInt && mod10 >= 2 && mod10 <= 4 && (mod100 < 12 || mod100 > 14)) return "few";
      if (isInt && ((i !== 1 && mod10 <= 1) || (mod10 >= 5 && mod10 <= 9) || (mod100 >= 12 && mod100 <= 14))) return "many";
      return "other";
    }
    if (lang === "cs" || lang === "sk") {
      if (isInt && i === 1) return "one";
      if (isInt && i >= 2 && i <= 4) return "few";
      if (!isInt) return "many";
      return "other";
    }
    if (lang === "fr" || lang === "pt") {
      return (v >= 0 && v < 2) ? "one" : "other";
    }
    if (lang === "ja" || lang === "zh" || lang === "ko" || lang === "vi" || lang === "th" || lang === "id" || lang === "tr") {
      return "other";
    }
    return (isInt && i === 1) ? "one" : "other";
  };
  PluralRules.prototype.resolvedOptions = function() {
    var lang = this._locale.split("-")[0];
    var cats = ["one", "other"];
    if (this._type === "ordinal") {
      cats = lang === "en" ? ["one", "two", "few", "other"] : ["other"];
    } else if (lang === "ar") {
      cats = ["zero", "one", "two", "few", "many", "other"];
    } else if (lang === "ru" || lang === "uk" || lang === "pl" || lang === "cs" || lang === "sk") {
      cats = ["one", "few", "many", "other"];
    } else if (lang === "ja" || lang === "zh" || lang === "ko" || lang === "vi" || lang === "th" || lang === "id" || lang === "tr") {
      cats = ["other"];
    }
    return {
      locale: this._locale,
      type: this._type,
      pluralCategories: cats
    };
  };
  PluralRules.supportedLocalesOf = DateTimeFormat.supportedLocalesOf;

  function __isGraphemeExtend(cp) {
    return (cp >= 0x0300 && cp <= 0x036f) ||
      (cp >= 0x0483 && cp <= 0x0489) ||
      (cp >= 0x0591 && cp <= 0x05c7) ||
      (cp >= 0x0610 && cp <= 0x061a) ||
      (cp >= 0x064b && cp <= 0x0670) ||
      (cp >= 0x0900 && cp <= 0x0903) ||
      (cp >= 0x093a && cp <= 0x094f) ||
      (cp >= 0x0951 && cp <= 0x0957) ||
      (cp >= 0x0962 && cp <= 0x0963) ||
      (cp >= 0x0e31 && cp <= 0x0e3a) ||
      (cp >= 0x0e47 && cp <= 0x0e4e) ||
      (cp >= 0x1160 && cp <= 0x11ff) ||
      (cp >= 0x1ab0 && cp <= 0x1aff) ||
      (cp >= 0x1dc0 && cp <= 0x1dff) ||
      cp === 0x200c || cp === 0x200d ||
      (cp >= 0x20d0 && cp <= 0x20ff) ||
      (cp >= 0xfe00 && cp <= 0xfe0f) ||
      (cp >= 0xfe20 && cp <= 0xfe2f) ||
      (cp >= 0x1f3fb && cp <= 0x1f3ff) ||
      (cp >= 0xe0020 && cp <= 0xe007f) ||
      (cp >= 0xe0100 && cp <= 0xe01ef);
  }

  function __isRegionalIndicator(cp) {
    return cp >= 0x1f1e6 && cp <= 0x1f1ff;
  }

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
      var reW = /([A-Za-z0-9_\u00C0-\u024F]+|[^\sA-Za-z0-9_\u00C0-\u024F]+|\s+)/g, mW;
      while ((mW = reW.exec(str)) !== null) {
        var segW = mW[0];
        list.push({
          segment: segW,
          index: mW.index,
          input: str,
          isWordLike: /[A-Za-z0-9_\u00C0-\u024F]/.test(segW)
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
      var cps = Array.from(str);
      var idx = 0;
      var i = 0;
      while (i < cps.length) {
        var ch = cps[i];
        var startIdx = idx;
        var cp = ch.codePointAt(0);
        idx += ch.length;
        i++;
        if (cp === 13 && i < cps.length && cps[i].codePointAt(0) === 10) {
          ch += cps[i];
          idx += cps[i].length;
          i++;
        } else {
          var prevCp = cp;
          var riCount = __isRegionalIndicator(cp) ? 1 : 0;
          while (i < cps.length) {
            var nextCh = cps[i];
            var nextCp = nextCh.codePointAt(0);
            if (__isGraphemeExtend(nextCp) || prevCp === 0x200d) {
              ch += nextCh;
              idx += nextCh.length;
              prevCp = nextCp;
              i++;
            } else if (__isRegionalIndicator(nextCp) && riCount % 2 === 1) {
              ch += nextCh;
              idx += nextCh.length;
              prevCp = nextCp;
              riCount++;
              i++;
            } else {
              break;
            }
          }
        }
        list.push({ segment: ch, index: startIdx, input: str });
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
    var o = (options && (options.timeStyle || options.dateStyle)) ? options : Object.assign({ hour: "numeric", minute: "2-digit", second: "2-digit" }, options || {});
    return new DateTimeFormat(locales, o).format(this);
  };
  Date.prototype.toLocaleString = function(locales, options) {
    var o = (options && (options.dateStyle || options.timeStyle)) ? options : Object.assign({ year: "numeric", month: "numeric", day: "numeric", hour: "numeric", minute: "2-digit", second: "2-digit" }, options || {});
    return new DateTimeFormat(locales, o).format(this);
  };
  Number.prototype.toLocaleString = function(locales, options) {
    return new NumberFormat(locales, options).format(this);
  };
  var __intlStrClean = typeof __slice_capture_ses_sigs === "function" ? __slice_capture_ses_sigs(1) : false;
  String.prototype.localeCompare = function(that, locales, options) {
    return new Collator(locales, options).compare(String(this), String(that));
  };
  if (__intlStrClean) __slice_capture_ses_sigs(2);
})();
