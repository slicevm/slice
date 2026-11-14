// ============================================================================
// SHARD 9: WEBASSEMBLY (W3C / WinterTC Standard API Backed by Pure-Rust wasmi)
// ============================================================================
(function() {
  function __makeWasmErrorCtor(name) {
    function WasmErr(message) {
      var err = new Error(message !== undefined ? String(message) : "");
      err.name = name;
      Object.setPrototypeOf(err, WasmErr.prototype);
      return err;
    }
    WasmErr.prototype = Object.create(Error.prototype);
    WasmErr.prototype.constructor = WasmErr;
    WasmErr.prototype.name = name;
    return WasmErr;
  }

  var CompileError = __makeWasmErrorCtor("CompileError");
  var LinkError = __makeWasmErrorCtor("LinkError");
  var RuntimeError = __makeWasmErrorCtor("RuntimeError");

  function Memory(descriptor) {
    if (!(this instanceof Memory)) {
      throw new TypeError("WebAssembly.Memory must be called with new");
    }
    this._buf = null;
    this._memId = __slice_wasm_memory_create(descriptor, this);
  }

  Object.defineProperty(Memory.prototype, "buffer", {
    get: function() {
      if (!this || typeof this._memId !== "number") {
        throw new TypeError("WebAssembly.Memory.prototype.buffer called on incompatible receiver");
      }
      var b = this._buf;
      if (b === null || b === undefined || b.detached === true) {
        b = __slice_wasm_memory_buffer(this._memId);
        this._buf = b;
      }
      return b;
    },
    enumerable: true,
    configurable: true
  });

  Memory.prototype.grow = function(delta) {
    if (!this || typeof this._memId !== "number") {
      throw new TypeError("WebAssembly.Memory.prototype.grow called on incompatible receiver");
    }
    if (arguments.length < 1) {
      throw new TypeError("WebAssembly.Memory.grow(): Argument 0 is required");
    }
    var prevPages = __slice_wasm_memory_grow(this._memId, delta);
    this._buf = null;
    return prevPages;
  };

  function Global(descriptor, v) {
    if (!(this instanceof Global)) {
      throw new TypeError("WebAssembly.Global must be called with new");
    }
    this._globId = __slice_wasm_global_create(descriptor, v, arguments.length >= 2, this);
  }

  Object.defineProperty(Global.prototype, "value", {
    get: function() {
      if (!this || typeof this._globId !== "number") {
        throw new TypeError("WebAssembly.Global.prototype.value called on incompatible receiver");
      }
      return __slice_wasm_global_get(this._globId);
    },
    set: function(nv) {
      if (!this || typeof this._globId !== "number") {
        throw new TypeError("WebAssembly.Global.prototype.value called on incompatible receiver");
      }
      if (arguments.length < 1) {
        throw new TypeError("WebAssembly.Global.prototype.value setter requires 1 argument");
      }
      __slice_wasm_global_set(this._globId, nv);
    },
    enumerable: true,
    configurable: true
  });

  Global.prototype.valueOf = function() {
    return this.value;
  };

  function Table(descriptor, fill) {
    if (!(this instanceof Table)) {
      throw new TypeError("WebAssembly.Table must be called with new");
    }
    this._tblId = __slice_wasm_table_create(descriptor, fill, arguments.length >= 2, this);
  }

  Object.defineProperty(Table.prototype, "length", {
    get: function() {
      if (!this || typeof this._tblId !== "number") {
        throw new TypeError("WebAssembly.Table.prototype.length called on incompatible receiver");
      }
      return __slice_wasm_table_length(this._tblId);
    },
    enumerable: true,
    configurable: true
  });

  Table.prototype.get = function(idx) {
    if (!this || typeof this._tblId !== "number") {
      throw new TypeError("WebAssembly.Table.prototype.get called on incompatible receiver");
    }
    if (arguments.length < 1) {
      throw new TypeError("WebAssembly.Table.get(): Argument 0 is required");
    }
    return __slice_wasm_table_get(this._tblId, idx);
  };

  Table.prototype.set = function(idx, val) {
    if (!this || typeof this._tblId !== "number") {
      throw new TypeError("WebAssembly.Table.prototype.set called on incompatible receiver");
    }
    if (arguments.length < 1) {
      throw new TypeError("WebAssembly.Table.set(): Argument 0 is required");
    }
    __slice_wasm_table_set(this._tblId, idx, val, arguments.length >= 2);
  };

  Table.prototype.grow = function(delta, fill) {
    if (!this || typeof this._tblId !== "number") {
      throw new TypeError("WebAssembly.Table.prototype.grow called on incompatible receiver");
    }
    if (arguments.length < 1) {
      throw new TypeError("WebAssembly.Table.grow(): Argument 0 is required");
    }
    return __slice_wasm_table_grow(this._tblId, delta, fill, arguments.length >= 2);
  };

  function Module(bytes) {
    if (!(this instanceof Module)) {
      throw new TypeError("WebAssembly.Module must be called with new");
    }
    if (!bytes || (typeof bytes !== "object" && typeof bytes !== "function")) {
      throw new TypeError("WebAssembly.Module(): Argument 0 must be a buffer source");
    }
    this._modId = __slice_wasm_compile(bytes);
  }

  Module.exports = function(mod) {
    if (!(mod instanceof Module) || typeof mod._modId !== "number") {
      throw new TypeError("WebAssembly.Module.exports(): Argument 0 must be a WebAssembly.Module");
    }
    return JSON.parse(__slice_wasm_module_exports(mod._modId));
  };

  Module.imports = function(mod) {
    if (!(mod instanceof Module) || typeof mod._modId !== "number") {
      throw new TypeError("WebAssembly.Module.imports(): Argument 0 must be a WebAssembly.Module");
    }
    return JSON.parse(__slice_wasm_module_imports(mod._modId));
  };

  Module.customSections = function(mod, sectionName) {
    if (!(mod instanceof Module) || typeof mod._modId !== "number") {
      throw new TypeError("WebAssembly.Module.customSections(): Argument 0 must be a WebAssembly.Module");
    }
    if (arguments.length < 2) {
      throw new TypeError("WebAssembly.Module.customSections(): Argument 1 is required");
    }
    return __slice_wasm_module_custom_sections(mod._modId, String(sectionName));
  };

  function Instance(mod, importObject) {
    if (!(this instanceof Instance)) {
      throw new TypeError("WebAssembly.Instance must be called with new");
    }
    if (!(mod instanceof Module) || typeof mod._modId !== "number") {
      throw new TypeError("WebAssembly.Instance(): Argument 0 must be a WebAssembly.Module");
    }
    if (importObject !== undefined && (typeof importObject !== "object" || importObject === null)) {
      throw new TypeError("WebAssembly.Instance(): Argument 1 must be an object or undefined");
    }
    this._exports = Object.freeze(__slice_wasm_instantiate(mod._modId, importObject));
  }

  Object.defineProperty(Instance.prototype, "exports", {
    get: function() {
      if (!(this instanceof Instance) || !this._exports) {
        throw new TypeError("WebAssembly.Instance.prototype.exports called on incompatible receiver");
      }
      return this._exports;
    },
    enumerable: true,
    configurable: true
  });

  function validate(bytes) {
    if (!bytes || (typeof bytes !== "object" && typeof bytes !== "function")) {
      throw new TypeError("WebAssembly.validate(): Argument 0 must be a buffer source");
    }
    try {
      return Boolean(__slice_wasm_validate(bytes));
    } catch (_) {
      return false;
    }
  }

  function compile(bytes) {
    return new Promise(function(resolve, reject) {
      try {
        resolve(new Module(bytes));
      } catch (e) {
        reject(e);
      }
    });
  }

  function instantiate(bytesOrModule, importObject) {
    return new Promise(function(resolve, reject) {
      try {
        if (bytesOrModule instanceof Module) {
          resolve(new Instance(bytesOrModule, importObject));
        } else {
          var mod = new Module(bytesOrModule);
          var inst = new Instance(mod, importObject);
          resolve({ module: mod, instance: inst });
        }
      } catch (e) {
        reject(e);
      }
    });
  }

  function compileStreaming(source) {
    return Promise.resolve(source)
      .then(function(resp) {
        if (resp && typeof resp.arrayBuffer === "function") {
          return resp.arrayBuffer();
        }
        return resp;
      })
      .then(function(buf) {
        return compile(buf);
      });
  }

  function instantiateStreaming(source, importObject) {
    return Promise.resolve(source)
      .then(function(resp) {
        if (resp && typeof resp.arrayBuffer === "function") {
          return resp.arrayBuffer();
        }
        return resp;
      })
      .then(function(buf) {
        return instantiate(buf, importObject);
      });
  }

  globalThis.WebAssembly = {
    Module: Module,
    Instance: Instance,
    Memory: Memory,
    Global: Global,
    Table: Table,
    CompileError: CompileError,
    LinkError: LinkError,
    RuntimeError: RuntimeError,
    validate: validate,
    compile: compile,
    instantiate: instantiate,
    compileStreaming: compileStreaming,
    instantiateStreaming: instantiateStreaming
  };
})();
