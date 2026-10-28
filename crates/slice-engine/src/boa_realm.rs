//! Warm `boa_engine` Realm Pool, Thread-Local Compartment State, `CORE_PRELUDE` WinterTC/SSE/Fetch Bootstrap,
//! and Native Host Intrinsics (`node:*` & `slice:test` lazy loader).

use crate::intrinsics::{
    base64_decode, base64_encode, fast_format_bytes, fast_format_ms, fast_parse_bytes,
    fast_parse_ms, hmac_sha256_hex, semver_coerce_xyz, semver_parse_strict_xyz,
    semver_satisfies_xyz, sha256_hex,
};
use crate::isolate::{SandboxEvalOptions, SliceIsolate};
use crate::outbound_http::{
    check_boa_outbound_fetch_ocap_and_dlp, execute_outbound_http11, parse_outbound_http_url,
};
use crate::state_and_kv::{
    checkpoint_tenant_state, edge_kv_del, edge_kv_get, edge_kv_incr, edge_kv_set,
    restore_tenant_state, splitmix64_next_f64, StateCheckpoint,
};
use crate::value_and_ast::BytecodeModule;
use rustc_hash::{FxHashMap, FxHashSet};
use slice_sandbox::{CapabilityToken, SandboxViolation};

#[derive(Default)]
pub(crate) struct ActiveBoaHttpReq {
    pub(crate) active: bool,
    pub(crate) method: String,
    pub(crate) path: String,
    pub(crate) query: String,
    pub(crate) tenant_id: String,
    pub(crate) body: String,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) headers_len: usize,
}

pub(crate) struct WarmBoaRealm {
    pub(crate) ctx: boa_engine::Context,
    pub(crate) loaded_bundles: FxHashMap<u64, boa_engine::JsObject>,
}

pub(crate) type ActiveBoaCaps = (String, Vec<String>, Vec<String>);

thread_local! {
    pub(crate) static SNP_MODULE_CACHE: std::cell::RefCell<FxHashMap<u64, std::sync::Arc<BytecodeModule>>> =
        std::cell::RefCell::new(FxHashMap::default());
    pub(crate) static BOA_STDOUT: std::cell::RefCell<Vec<String>> =
        std::cell::RefCell::new(Vec::with_capacity(16));
    pub(crate) static BOA_CONTEXT_POOL: std::cell::RefCell<std::mem::ManuallyDrop<Vec<WarmBoaRealm>>> =
        const { std::cell::RefCell::new(std::mem::ManuallyDrop::new(Vec::new())) };
    pub(crate) static BOA_VFS_CACHE: std::cell::RefCell<FxHashMap<String, String>> =
        std::cell::RefCell::new(FxHashMap::default());
    pub(crate) static BOA_VFS_DIRS: std::cell::RefCell<FxHashSet<String>> =
        std::cell::RefCell::new(FxHashSet::default());
    pub(crate) static BOA_ACTIVE_CAPS: std::cell::RefCell<Option<ActiveBoaCaps>> =
        const { std::cell::RefCell::new(None) };
    pub(crate) static BOA_VIOLATION: std::cell::RefCell<Option<SandboxViolation>> =
        const { std::cell::RefCell::new(None) };
    pub(crate) static BOA_HTTP_REQ: std::cell::RefCell<ActiveBoaHttpReq> =
        std::cell::RefCell::new(ActiveBoaHttpReq::default());
    pub(crate) static BOA_HTTP_RES: std::cell::RefCell<Option<(u16, String, String)>> =
        const { std::cell::RefCell::new(None) };
    pub(crate) static BOA_WAIT_UNTIL_COUNT: std::cell::Cell<usize> =
        const { std::cell::Cell::new(0) };
    pub(crate) static BOA_ACTIVE_TENANT: std::cell::RefCell<String> =
        std::cell::RefCell::new(String::with_capacity(32));
    pub(crate) static BOA_KV_MEM_BUDGET: std::cell::Cell<(usize, usize)> =
        const { std::cell::Cell::new((0, usize::MAX)) };
    pub(crate) static BOA_DET_STATE: std::cell::Cell<Option<(u64, u64)>> =
        const { std::cell::Cell::new(None) };
    pub(crate) static BOA_NET_HOSTS: std::cell::RefCell<FxHashSet<String>> =
        std::cell::RefCell::new(FxHashSet::default());
    pub(crate) static BOA_EGRESS_SECRETS: std::cell::RefCell<Vec<String>> =
        const { std::cell::RefCell::new(Vec::new()) };
    pub(crate) static BOA_ENV_MAP: std::cell::RefCell<FxHashMap<String, String>> =
        std::cell::RefCell::new(FxHashMap::default());
    pub(crate) static BOA_FETCH_MOCKS: std::cell::RefCell<FxHashMap<String, String>> =
        std::cell::RefCell::new(FxHashMap::default());
    pub(crate) static CHECKPOINT_SLAB: std::cell::RefCell<FxHashMap<u64, StateCheckpoint>> =
        std::cell::RefCell::new(FxHashMap::default());
    pub(crate) static NEXT_CHECKPOINT_ID: std::cell::Cell<u64> =
        const { std::cell::Cell::new(1) };
}

#[inline]
pub(crate) fn norm_vfs_path(p: &str) -> String {
    let mut s = if p.as_bytes().contains(&b'\\') {
        p.replace('\\', "/")
    } else {
        p.to_string()
    };
    while s.len() > 1 && s.ends_with('/') {
        s.pop();
    }
    s
}

#[cold]
#[inline(never)]
pub(crate) fn record_boa_fs_violation(tenant_id: &str, path: &str) -> bool {
    BOA_VIOLATION.with(|v| {
        *v.borrow_mut() = Some(SandboxViolation::FsReadDenied {
            tenant_id: tenant_id.to_string(),
            path: path.to_string(),
        });
    });
    false
}

#[inline]
pub(crate) fn check_boa_fs_read_ocap(path: &str) -> bool {
    if path.contains("..") {
        let tenant = BOA_ACTIVE_CAPS
            .with(|c| c.borrow().as_ref().map(|t| t.0.clone()))
            .unwrap_or_else(|| "boa-tenant".to_string());
        return record_boa_fs_violation(&tenant, path);
    }
    BOA_ACTIVE_CAPS.with(|c| {
        if let Some((tenant_id, read_prefixes, _)) = &*c.borrow() {
            if !read_prefixes.is_empty()
                && !read_prefixes.iter().any(|prefix| path.starts_with(prefix))
            {
                return record_boa_fs_violation(tenant_id, path);
            }
        }
        true
    })
}

#[inline]
pub(crate) fn check_boa_fs_write_ocap(path: &str) -> bool {
    if path.contains("..") {
        let tenant = BOA_ACTIVE_CAPS
            .with(|c| c.borrow().as_ref().map(|t| t.0.clone()))
            .unwrap_or_else(|| "boa-tenant".to_string());
        return record_boa_fs_violation(&tenant, path);
    }
    BOA_ACTIVE_CAPS.with(|c| {
        if let Some((tenant_id, _, write_prefixes)) = &*c.borrow() {
            if !write_prefixes.is_empty()
                && !write_prefixes.iter().any(|prefix| path.starts_with(prefix))
            {
                return record_boa_fs_violation(tenant_id, path);
            }
        }
        true
    })
}

pub(crate) fn acquire_warm_boa_realm() -> WarmBoaRealm {
    BOA_CONTEXT_POOL
        .with(|pool| pool.borrow_mut().pop())
        .unwrap_or_else(create_warm_boa_realm)
}

pub(crate) fn release_warm_boa_realm(realm: WarmBoaRealm) {
    BOA_CONTEXT_POOL.with(|pool| {
        let mut p = pool.borrow_mut();
        if p.len() < 8 {
            p.push(realm);
        }
    });
}

/// Phase 9B Ultra-Compact Lazy Module Injection (`< 480 bytes` core bootstrap):
/// Eliminates the `~2.2 ms` cold-start `PRELUDE` parser tax by binding `console.log` directly to
/// the native Rust variadic callable and loading `node:crypto`, `node:fs`, `node:path`, `node:os`,
/// and `node:util` on demand via `__slice_load_builtin(id)` only when a tenant actually requires them.
pub(crate) fn create_warm_boa_realm() -> WarmBoaRealm {
    let mut ctx = boa_engine::Context::default();
    register_slice_native_host_functions(&mut ctx);

    const CORE_PRELUDE: &str = r##"globalThis.console={log:__slice_console_log,info:__slice_console_log,warn:__slice_console_log,error:__slice_console_log};globalThis.print=__slice_console_log;globalThis.env=function(k){return __slice_env_get(String(k));};globalThis.process={pid:1,env:{NODE_ENV:"production"},cwd:function(){return ".";},platform:"linux",arch:"x64",version:"v24.0.0",versions:{node:"24.0.0"},argv:["slice"]};globalThis.__slice_sync_process_env=function(load){if(!load){globalThis.process.env={NODE_ENV:"production"};return;}var m=JSON.parse(__slice_env_json());m.NODE_ENV=m.NODE_ENV||"production";globalThis.process.env=m;};var __b2s=function(e){return e==="base64"?__slice_base64_encode(this._s,false):e==="base64url"?__slice_base64_encode(this._s,true):this._s;};globalThis.Buffer={from:function(i,e){var r=typeof i==="string"?i:String(i),s=(e==="base64"||e==="base64url")?__slice_base64_decode(r):r;return{_s:s,length:s.length,toString:__b2s};},isBuffer:function(){return false;},byteLength:function(s){return String(s).length;}};var __orand=Math.random,__onow=Date.now;Math.random=function(){var v=__slice_det_random();return v>=0?v:__orand();};Date.now=function(){var v=__slice_det_now();return v>=0?v:__onow();};globalThis.performance={now:function(){var v=__slice_det_now();return v>=0?(v-1700000000000):(__onow()-1700000000000);}};function __fmtSse(ev){if(ev===undefined||ev===null)return "";if(typeof ev==="string"){if(ev.indexOf("\n")===-1)return "data: "+ev+"\n\n";var ls=ev.split("\n"),o="";for(var i=0;i<ls.length;i++)o+="data: "+ls[i]+"\n";return o+"\n";}var s="";if(ev.id!==undefined&&ev.id!==null)s+="id: "+String(ev.id)+"\n";if(ev.event!==undefined&&ev.event!==null)s+="event: "+String(ev.event)+"\n";if(ev.retry!==undefined&&ev.retry!==null)s+="retry: "+Number(ev.retry)+"\n";var d=ev.data!==undefined?(typeof ev.data==="string"?ev.data:JSON.stringify(ev.data)):JSON.stringify(ev);if(d.indexOf("\n")===-1)return s+"data: "+d+"\n\n";var dl=d.split("\n");for(var j=0;j<dl.length;j++)s+="data: "+dl[j]+"\n";return s+"\n";}function __colSse(src){var out="";if(Array.isArray(src)){for(var i=0;i<src.length;i++)out+=__fmtSse(src[i]);}else if(typeof src==="function"){var r=src(function(ev){out+=__fmtSse(ev);});if(r&&typeof r.next==="function"){var st=r.next();while(!st.done){out+=__fmtSse(st.value);st=r.next();}}else if(Array.isArray(r)){for(var j=0;j<r.length;j++)out+=__fmtSse(r[j]);}}else if(src&&typeof src.next==="function"){var s2=src.next();while(!s2.done){out+=__fmtSse(s2.value);s2=src.next();}}else if(src!==undefined&&src!==null){out+=__fmtSse(src);}return out;}function ReadableStream(src){var chunks=[],closed=false;this._chunks=chunks;if(src&&typeof src.start==="function"){src.start({enqueue:function(c){if(!closed)chunks.push(typeof c==="string"?c:String(c));},close:function(){closed=true;}});}}ReadableStream.prototype._toText=function(){return this._chunks.join("");};ReadableStream.prototype.getReader=function(){var cs=this._chunks,idx=0;return{read:function(){if(idx<cs.length)return Promise.resolve({value:cs[idx++],done:false});return Promise.resolve({value:undefined,done:true});},releaseLock:function(){}};};globalThis.ReadableStream=ReadableStream;function __doFetch(input,init){var req=new Request(input,init),raw=JSON.parse(__slice_fetch_sync(req.url,req.method,JSON.stringify(req.headers.entries()),req._body));if(raw.__error)throw new Error(raw.__error);var r=new Response(raw.body,{status:raw.status,statusText:raw.statusText,headers:raw.headers});r.url=req.url;return r;}globalThis.fetch=function(input,init){try{return Promise.resolve(__doFetch(input,init));}catch(e){return Promise.reject(e);}};function __tMod(){return __slice_builtins["slice:test"]||__slice_load_builtin("slice:test");}globalThis.describe=function(n,f){return __tMod().describe(n,f);};globalThis.test=globalThis.it=function(n,f,o){return __tMod().test(n,f,o);};globalThis.beforeEach=function(f){return __tMod().beforeEach(f);};globalThis.afterEach=function(f){return __tMod().afterEach(f);};globalThis.expect=function(v){return __tMod().expect(v);};globalThis.__slice_wu=[];globalThis.Slice={request:function(){var t=__slice_http_tenant();return{method:__slice_http_method(),path:__slice_http_path(),query:__slice_http_query(),tenant:t,tenantId:t,body:__slice_http_body(),header:__slice_http_header};},respond:function(s,b,ct){return __slice_http_respond(s,typeof b==="string"?b:JSON.stringify(b),ct||"application/json");},streamSSE:function(src,init){var r=Response.sse(src,init);__slice_http_respond(r.status,r._body,r._ct);return r;},fetchSync:__doFetch,mockFetch:function(a,b,c){var m="*",u=String(a),o=b||{};if(c!==undefined){m=String(a).toUpperCase();u=String(b);o=c||{};}var h=new Headers(o.headers),bd=o.body===undefined||o.body===null?"":(typeof o.body==="string"?o.body:JSON.stringify(o.body));if(typeof o.body==="object"&&o.body!==null&&!h._m["content-type"])h._m["content-type"]="application/json";__slice_mock_fetch(m==="*"?u:(m+" "+u),JSON.stringify({status:o.status||200,statusText:o.statusText||"OK",headers:h.entries(),body:bd}));},waitUntil:function(f){__slice_wu.push(f);__slice_mark_wait_until();},kv:{get:function(k){return __slice_kv_get(String(k));},set:function(k,v,ttl){return __slice_kv_set(String(k),typeof v==="string"?v:JSON.stringify(v),ttl||0);},del:function(k){return __slice_kv_del(String(k));},incr:function(k,d){return __slice_kv_incr(String(k),d===undefined?1:Number(d));}},checkpoint:function(){return __slice_checkpoint();},restore:function(id){return __slice_restore(Number(id));},evalSandbox:function(code,opts){return JSON.parse(__slice_eval_sandbox(String(code),opts?JSON.stringify(opts):"{}"));},testSummary:function(){return __tMod().summary();},resetTests:function(){return __tMod().reset();}};function __uspD(s){if(s.indexOf("+")===-1&&s.indexOf("%")===-1)return s;var r=s.indexOf("+")===-1?s:s.split("+").join(" ");if(r.indexOf("%")===-1)return r;try{return decodeURIComponent(r);}catch(_e){return r;}}function __uspE(s){var str=String(s),e=encodeURIComponent(str);return str.indexOf(" ")===-1?e:e.split("%20").join("+");}function URLSearchParams(init,u){this._p=[];this._u=u||null;if(!init)return;if(typeof init==="string"){var q=init.charCodeAt(0)===63?init.slice(1):init;if(q.length>0){var ps=q.split("&");for(var i=0;i<ps.length;i++){var sg=ps[i];if(!sg)continue;var eq=sg.indexOf("=");if(eq===-1)this._p.push([__uspD(sg),""]);else this._p.push([__uspD(sg.slice(0,eq)),__uspD(sg.slice(eq+1))]);}}}else if(Array.isArray(init)){for(var j=0;j<init.length;j++){var pr=init[j];if(pr&&pr.length>=2)this._p.push([String(pr[0]),String(pr[1])]);}}else if(typeof init==="object"){if(Array.isArray(init._p)){for(var m=0;m<init._p.length;m++)this._p.push([init._p[m][0],init._p[m][1]]);}else{var ks=Object.keys(init);for(var k=0;k<ks.length;k++)this._p.push([ks[k],String(init[ks[k]])]);}}}URLSearchParams.prototype._sync=function(){if(this._u)this._u._syncSearch();};URLSearchParams.prototype.get=function(n){var k=String(n);for(var i=0;i<this._p.length;i++)if(this._p[i][0]===k)return this._p[i][1];return null;};URLSearchParams.prototype.getAll=function(n){var k=String(n),r=[];for(var i=0;i<this._p.length;i++)if(this._p[i][0]===k)r.push(this._p[i][1]);return r;};URLSearchParams.prototype.has=function(n){var k=String(n);for(var i=0;i<this._p.length;i++)if(this._p[i][0]===k)return true;return false;};URLSearchParams.prototype.set=function(n,v){var k=String(n),s=String(v),p=this._p,len=p.length,fi=-1;for(var i=0;i<len;i++){if(p[i][0]===k){if(fi===-1){p[i][1]=s;fi=i;}else{p.splice(i,1);i--;len--;}}}if(fi===-1)p.push([k,s]);if(this._u)this._u._syncSearch();};URLSearchParams.prototype.append=function(n,v){this._p.push([String(n),String(v)]);if(this._u)this._u._syncSearch();};URLSearchParams.prototype.delete=function(n){var k=String(n),nx=[];for(var i=0;i<this._p.length;i++)if(this._p[i][0]!==k)nx.push(this._p[i]);this._p=nx;if(this._u)this._u._syncSearch();};URLSearchParams.prototype.sort=function(){this._p.sort(function(a,b){return a[0]<b[0]?-1:(a[0]>b[0]?1:0);});if(this._u)this._u._syncSearch();};URLSearchParams.prototype.forEach=function(cb,ta){for(var i=0;i<this._p.length;i++)cb.call(ta,this._p[i][1],this._p[i][0],this);};URLSearchParams.prototype.entries=function(){return this._p.slice();};URLSearchParams.prototype.keys=function(){var r=[];for(var i=0;i<this._p.length;i++)r.push(this._p[i][0]);return r;};URLSearchParams.prototype.values=function(){var r=[];for(var i=0;i<this._p.length;i++)r.push(this._p[i][1]);return r;};URLSearchParams.prototype.toString=function(){var len=this._p.length;if(len===0)return "";var s="";for(var i=0;i<len;i++){if(i>0)s+="&";s+=__uspE(this._p[i][0])+"="+__uspE(this._p[i][1]);}return s;};if(typeof Symbol!=="undefined"&&Symbol.iterator)URLSearchParams.prototype[Symbol.iterator]=function(){return this._p[Symbol.iterator]();};Object.defineProperty(URLSearchParams.prototype,"size",{get:function(){return this._p.length;}});function __normP(p){if(!p||p==="/")return "/";if(p.indexOf("/.")===-1&&p.indexOf("//")===-1)return p;var segs=p.split("/"),out=[];for(var i=0;i<segs.length;i++){var s=segs[i];if(!s||s===".")continue;if(s==="..")out.pop();else out.push(s);}return "/"+out.join("/")+(p.length>1&&p.charCodeAt(p.length-1)===47&&out.length>0?"/":"");}function URL(input,base){var raw=input instanceof URL?input.href:String(input).trim();if(base!==undefined&&raw.indexOf("https://")!==0&&raw.indexOf("http://")!==0&&raw.indexOf("://")===-1){var b=base instanceof URL?base:new URL(String(base));if(raw.indexOf("//")===0)raw=b.protocol+raw;else if(raw.charCodeAt(0)===47)raw=b.origin+raw;else if(raw.charCodeAt(0)===63)raw=b.origin+b.pathname+raw;else if(raw.charCodeAt(0)===35)raw=b.origin+b.pathname+b.search+raw;else{var dir=b.pathname.slice(0,b.pathname.lastIndexOf("/")+1)||"/";raw=b.origin+dir+raw;}}var hi=raw.indexOf("#"),hash="";if(hi!==-1){hash=raw.slice(hi);raw=raw.slice(0,hi);}var qi=raw.indexOf("?"),search="";if(qi!==-1){search=raw.slice(qi);raw=raw.slice(0,qi);}var si=raw.indexOf("://");if(si===-1)throw new TypeError("Invalid URL: "+input);this.protocol=raw.slice(0,si+1).toLowerCase();var rest=raw.slice(si+3),pi=rest.indexOf("/"),authHost=pi===-1?rest:rest.slice(0,pi),pathRaw=pi===-1?"/":rest.slice(pi);var ai=authHost.indexOf("@"),hostPort=authHost;this.username="";this.password="";if(ai!==-1){var up=authHost.slice(0,ai),ci=up.indexOf(":");this.username=ci===-1?up:up.slice(0,ci);this.password=ci===-1?"":up.slice(ci+1);hostPort=authHost.slice(ai+1);}var pci=hostPort.lastIndexOf(":");if(pci!==-1&&hostPort.indexOf("]")<pci){this.hostname=hostPort.slice(0,pci).toLowerCase();var prt=hostPort.slice(pci+1);if((this.protocol==="http:"&&prt==="80")||(this.protocol==="https:"&&prt==="443"))prt="";this.port=prt;}else{this.hostname=hostPort.toLowerCase();this.port="";}this.host=this.port?this.hostname+":"+this.port:this.hostname;this.origin=this.protocol+"//"+this.host;this.pathname=__normP(pathRaw);this.hash=hash==="#"?"":hash;this.searchParams=new URLSearchParams(search,this);this._syncSearch();}URL.prototype._syncSearch=function(){if(this.searchParams._p.length===0){this.search="";this.href=this.origin+this.pathname+this.hash;return;}var qs=this.searchParams.toString();this.search=qs?"?"+qs:"";this.href=this.origin+this.pathname+this.search+this.hash;};URL.prototype.toString=URL.prototype.toJSON=function(){return this.href;};function Headers(init){if(init==="__SLICE_LIVE_REQ__"){this._live=true;this._m=null;return;}this._live=false;this._m=Object.create(null);if(!init)return;if(init instanceof Headers){if(init._m){var ik=Object.keys(init._m);for(var n=0;n<ik.length;n++)this._m[ik[n]]=init._m[ik[n]];}else{var es=init.entries();for(var i=0;i<es.length;i++)this._m[es[i][0]]=es[i][1];}}else if(Array.isArray(init)){for(var j=0;j<init.length;j++)if(init[j]&&init[j].length>=2)this.append(init[j][0],init[j][1]);}else if(typeof init==="object"){var ks=Object.keys(init);for(var k=0;k<ks.length;k++)if(init[ks[k]]!==undefined)this._m[ks[k].toLowerCase()]=String(init[ks[k]]);}}Headers.prototype._ensure=function(){if(this._live&&!this._m){this._m=Object.create(null);var raw=JSON.parse(__slice_http_headers_all());for(var i=0;i<raw.length;i++){var k=raw[i][0].toLowerCase();this._m[k]=this._m[k]?this._m[k]+", "+raw[i][1]:raw[i][1];}this._live=false;}};Headers.prototype.get=function(n){var k=String(n).toLowerCase();if(this._m&&k in this._m)return this._m[k];if(this._live)return __slice_http_header(k);return null;};Headers.prototype.has=function(n){return this.get(n)!==null;};Headers.prototype.set=function(n,v){this._ensure();this._m[String(n).toLowerCase()]=String(v);};Headers.prototype.append=function(n,v){this._ensure();var k=String(n).toLowerCase(),s=String(v);this._m[k]=k in this._m?this._m[k]+", "+s:s;};Headers.prototype.delete=function(n){this._ensure();delete this._m[String(n).toLowerCase()];};Headers.prototype.entries=function(){this._ensure();var ks=Object.keys(this._m).sort(),r=[];for(var i=0;i<ks.length;i++)r.push([ks[i],this._m[ks[i]]]);return r;};Headers.prototype.keys=function(){this._ensure();return Object.keys(this._m).sort();};Headers.prototype.values=function(){var es=this.entries(),r=[];for(var i=0;i<es.length;i++)r.push(es[i][1]);return r;};Headers.prototype.forEach=function(cb,ta){var es=this.entries();for(var i=0;i<es.length;i++)cb.call(ta,es[i][1],es[i][0],this);};if(typeof Symbol!=="undefined"&&Symbol.iterator)Headers.prototype[Symbol.iterator]=function(){return this.entries()[Symbol.iterator]();};function Request(input,init){if(input==="__SLICE_LIVE_REQ__"){this.method=__slice_http_method()||"GET";var h=__slice_http_header("host")||"localhost",p=__slice_http_path()||"/",q=__slice_http_query();this.url="http://"+h+p+(q?"?"+q:"");this.headers=new Headers("__SLICE_LIVE_REQ__");this._body=__slice_http_body()||"";this.bodyUsed=false;return;}var o=init||{},isReq=input instanceof Request;this.url=isReq?input.url:(input instanceof URL?input.href:String(input));this.method=o.method?String(o.method).toUpperCase():(isReq?input.method:"GET");this.headers=o.headers instanceof Headers?o.headers:new Headers(o.headers||(isReq?input.headers:undefined));this._body=o.body!==undefined&&o.body!==null?(typeof o.body==="string"?o.body:JSON.stringify(o.body)):(isReq?input._body:"");this.bodyUsed=false;}Request.prototype.text=function(){this.bodyUsed=true;return Promise.resolve(this._body);};Request.prototype.json=function(){this.bodyUsed=true;try{return Promise.resolve(JSON.parse(this._body));}catch(e){return Promise.reject(e);}};Request.prototype.textSync=function(){this.bodyUsed=true;return this._body;};Request.prototype.jsonSync=function(){this.bodyUsed=true;return JSON.parse(this._body);};Request.prototype.clone=function(){return new Request(this.url,{method:this.method,headers:new Headers(this.headers),body:this._body});};function Response(body,init){var o=init||{};this.status=o.status!==undefined?Number(o.status):200;this.statusText=o.statusText!==undefined?String(o.statusText):(this.status===200?"OK":"");this.ok=this.status>=200&&this.status<300;this.headers=o.headers instanceof Headers?o.headers:new Headers(o.headers);this._data=undefined;this._body=body===undefined||body===null?"":(body instanceof ReadableStream?body._toText():(typeof body==="string"?body:JSON.stringify(body)));this._ct=(this.headers._m&&this.headers._m["content-type"])||"text/plain;charset=UTF-8";this._isSliceRes=true;this.bodyUsed=false;}Response.prototype.text=function(){this.bodyUsed=true;if(this._data!==undefined&&!this._body)this._body=JSON.stringify(this._data);return Promise.resolve(this._body);};Response.prototype.json=function(){this.bodyUsed=true;if(this._data!==undefined)return Promise.resolve(this._data);try{return Promise.resolve(JSON.parse(this._body));}catch(e){return Promise.reject(e);}};Response.prototype.textSync=function(){this.bodyUsed=true;if(this._data!==undefined&&!this._body)this._body=JSON.stringify(this._data);return this._body;};Response.prototype.jsonSync=function(){this.bodyUsed=true;if(this._data!==undefined)return this._data;return JSON.parse(this._body);};Response.prototype.clone=function(){if(this._data!==undefined&&!this._body)this._body=JSON.stringify(this._data);return new Response(this._body,{status:this.status,statusText:this.statusText,headers:new Headers(this.headers)});};Response.json=function(data,init){var o=init?(typeof init==="number"?{status:init}:init):{},h=o.headers instanceof Headers?o.headers:new Headers(o.headers);if(!h._m["content-type"])h._m["content-type"]="application/json";var r=new Response(null,{status:o.status||200,statusText:o.statusText,headers:h});r._data=data;r._ct=h._m["content-type"];return r;};Response.sse=function(src,init){var o=init?(typeof init==="number"?{status:init}:init):{},h=o.headers instanceof Headers?o.headers:new Headers(o.headers);if(!h._m["content-type"])h._m["content-type"]="text/event-stream; charset=utf-8";if(!h._m["cache-control"])h._m["cache-control"]="no-cache";var r=new Response(__colSse(src),{status:o.status||200,statusText:o.statusText,headers:h});r._ct=h._m["content-type"];return r;};Response.redirect=function(url,status){var st=status||302;return new Response("",{status:st,headers:{location:String(url)}});};globalThis.URLSearchParams=URLSearchParams;globalThis.URL=URL;globalThis.Headers=Headers;globalThis.Request=Request;globalThis.Response=Response;var __slice_web_ctx={waitUntil:Slice.waitUntil,passThroughOnException:function(){}};globalThis.__slice_make_web_request=function(){return new Request("__SLICE_LIVE_REQ__");};globalThis.__slice_dispatch_fetch=function(h){if(!__slice_http_is_active())return;var fn=typeof h.fetch==="function"?h.fetch:(typeof h==="function"?h:null);if(!fn)return;var res=fn.call(h,new Request("__SLICE_LIVE_REQ__"),globalThis.process.env,__slice_web_ctx);if(res){if(res._isSliceRes){var b=res._body||(res._data!==undefined?JSON.stringify(res._data):"");__slice_http_respond(res.status,b,res._ct);}else if(typeof res.then==="function")res.then(function(r){if(r&&r._isSliceRes){var rb=r._body||(r._data!==undefined?JSON.stringify(r._data):"");__slice_http_respond(r.status,rb,r._ct);}});}};globalThis.__slice_builtins=Object.create(null);globalThis.__slice_cjs_require=function(id){return __slice_builtins[id]||__slice_load_builtin(id);};globalThis.require=__slice_cjs_require;"##;
    let _ = ctx.eval(boa_engine::Source::from_bytes(CORE_PRELUDE));
    WarmBoaRealm {
        ctx,
        loaded_bundles: FxHashMap::default(),
    }
}

pub(crate) fn register_slice_native_host_functions(ctx: &mut boa_engine::Context) {
    use boa_engine::{js_string, JsValue as BoaVal, NativeFunction};

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_console_log"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let line = if args.len() == 1 {
                let first = &args[0];
                if let Some(s) = first.as_string() {
                    s.to_std_string_escaped()
                } else {
                    first.to_string(context)?.to_std_string_escaped()
                }
            } else if args.is_empty() {
                String::new()
            } else {
                let mut joined = String::with_capacity(64);
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 {
                        joined.push(' ');
                    }
                    if let Some(s) = arg.as_string() {
                        joined.push_str(&s.to_std_string_escaped());
                    } else {
                        joined.push_str(&arg.to_string(context)?.to_std_string_escaped());
                    }
                }
                joined
            };
            BOA_STDOUT.with(|out| out.borrow_mut().push(line));
            Ok(BoaVal::undefined())
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_load_builtin"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let id = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let snippet: &[u8] = match id.as_str() {
                "crypto" | "node:crypto" => br#"var __h={_c:"",_b:false,update:function(d){this._c+=String(d);return this;},digest:function(){var r=__slice_crypto_sha256(this._c);this._c="";this._b=false;return r;}},__m={_k:"",_c:"",_b:false,update:function(d){this._c+=String(d);return this;},digest:function(){var r=__slice_crypto_hmac_sha256(this._k,this._c);this._c="";this._b=false;return r;}},m={createHash:function(){if(!__h._b){__h._c="";__h._b=true;return __h;}return{_c:"",update:function(d){this._c+=String(d);return this;},digest:function(){return __slice_crypto_sha256(this._c);}};},createHmac:function(_a,s){var k=String(s);if(!__m._b){__m._k=k;__m._c="";__m._b=true;return __m;}return{_c:"",update:function(d){this._c+=String(d);return this;},digest:function(){return __slice_crypto_hmac_sha256(k,this._c);}};}};m.default=m;m.__esModule=true;__slice_builtins["crypto"]=__slice_builtins["node:crypto"]=m;"#,
                "fs" | "node:fs" => br#"var m={readFileSync:function(p){return __slice_fs_read_file_sync(String(p));},writeFileSync:function(p,d){return __slice_fs_write_file_sync(String(p),String(d));},appendFileSync:function(p,d){return __slice_fs_append_file_sync(String(p),String(d));},existsSync:function(p){return Boolean(__slice_fs_exists_sync(String(p)));},mkdirSync:function(p){return __slice_fs_mkdir_sync(String(p));},readdirSync:function(p){return JSON.parse(__slice_fs_readdir_sync(String(p)));},statSync:function(p){return{size:__slice_fs_stat_sync(String(p))};},unlinkSync:function(p){return __slice_fs_rm_sync(String(p));},rmSync:function(p){return __slice_fs_rm_sync(String(p));}};m.default=m;m.__esModule=true;__slice_builtins["fs"]=__slice_builtins["node:fs"]=m;"#,
                "path" | "node:path" => br#"var m={sep:"/",join:function(){return __slice_path_join.apply(null,arguments);},resolve:function(){return __slice_path_join.apply(null,arguments);},basename:function(p,ext){var s=String(p).replace(/\\/g,"/").replace(/\/+$/,""),i=s.lastIndexOf("/"),b=i>=0?s.slice(i+1):s;return(ext&&b.endsWith(ext))?b.slice(0,b.length-ext.length):b;},dirname:function(p){var s=String(p).replace(/\\/g,"/").replace(/\/+$/,""),i=s.lastIndexOf("/");return i>0?s.slice(0,i):(i===0?"/":".");},extname:function(p){var b=m.basename(p),i=b.lastIndexOf(".");return i>0?b.slice(i):"";}};m.default=m;m.__esModule=true;__slice_builtins["path"]=__slice_builtins["node:path"]=m;"#,
                "os" | "node:os" => br#"var m={tmpdir:function(){return __slice_os_tmpdir();},platform:function(){return "linux";},arch:function(){return "x64";},EOL:"\n"};m.default=m;m.__esModule=true;__slice_builtins["os"]=__slice_builtins["node:os"]=m;"#,
                "util" | "node:util" => br#"var m={inspect:function(v){return typeof v==="object"?JSON.stringify(v):String(v);},deprecate:function(f){return f;},promisify:function(f){return f;}};m.default=m;__slice_builtins["util"]=__slice_builtins["node:util"]=m;"#,
                "buffer" | "node:buffer" => br#"var m={Buffer:globalThis.Buffer,default:{Buffer:globalThis.Buffer}};__slice_builtins["buffer"]=__slice_builtins["node:buffer"]=m;"#,
                "url" | "node:url" => br#"var m={URL:globalThis.URL,URLSearchParams:globalThis.URLSearchParams,default:{URL:globalThis.URL,URLSearchParams:globalThis.URLSearchParams}};m.__esModule=true;__slice_builtins["url"]=__slice_builtins["node:url"]=m;"#,
                "process" | "node:process" => br#"__slice_builtins["process"]=__slice_builtins["node:process"]=globalThis.process;"#,
                "slice:test" | "node:test" | "bun:test" | "test" => br#"var __st={suites:[],be:[[]],ae:[[]],res:[],passed:0,failed:0,skipped:0,rollbacks:0};function __eq(a,b){if(a===b)return true;if(a&&b&&typeof a==="object"&&typeof b==="object")return JSON.stringify(a)===JSON.stringify(b);return false;}function __vMatch(v,s){if(!s)return true;var q=String(s);if(v.indexOf(q)!==-1)return true;if(q==="FsReadDenied")return v.indexOf("filesystem read authority")!==-1;if(q==="FsWriteDenied")return v.indexOf("filesystem write authority")!==-1;if(q==="NetAccessDenied"||q==="NetConnectDenied")return v.indexOf("outbound network authority")!==-1;if(q==="EnvAccessDenied")return v.indexOf("environment variable authority")!==-1;if(q==="CpuFuelExhausted")return v.indexOf("CPU instruction fuel limit")!==-1;if(q==="MemoryCeilingExceeded"||q==="MemoryLimitExceeded")return v.indexOf("memory ceiling")!==-1;return false;}function __mkExp(act,neg){function chk(ok,msg){if(neg?ok:!ok)throw new Error(msg);}return{get not(){return __mkExp(act,!neg);},toBe:function(e){chk(act===e,"Expected "+JSON.stringify(e)+" but got "+JSON.stringify(act));},toEqual:function(e){chk(__eq(act,e),"Expected deep equal "+JSON.stringify(e)+" but got "+JSON.stringify(act));},toContain:function(s){var ok=(typeof act==="string"||Array.isArray(act))&&act.indexOf(s)!==-1;chk(ok,"Expected "+JSON.stringify(act)+" to contain "+JSON.stringify(s));},toBeTruthy:function(){chk(Boolean(act),"Expected truthy but got "+JSON.stringify(act));},toBeFalsy:function(){chk(!act,"Expected falsy but got "+JSON.stringify(act));},toBeNull:function(){chk(act===null,"Expected null but got "+JSON.stringify(act));},toBeDefined:function(){chk(act!==undefined,"Expected defined value");},toBeUndefined:function(){chk(act===undefined,"Expected undefined but got "+JSON.stringify(act));},toBeGreaterThan:function(n){chk(Number(act)>Number(n),"Expected "+act+" > "+n);},toBeGreaterThanOrEqual:function(n){chk(Number(act)>=Number(n),"Expected "+act+" >= "+n);},toBeLessThan:function(n){chk(Number(act)<Number(n),"Expected "+act+" < "+n);},toBeLessThanOrEqual:function(n){chk(Number(act)<=Number(n),"Expected "+act+" <= "+n);},toHaveLength:function(n){chk(act!=null&&act.length===n,"Expected length "+n+" but got "+(act&&act.length));},toHaveProperty:function(k,v){var hp=act!=null&&typeof act==="object"&&(k in act);if(hp&&v!==undefined)hp=__eq(act[k],v);chk(hp,"Expected property "+k);},toThrow:function(sub){var th=false,em="";try{act();}catch(e){th=true;em=String(e&&e.message||e);}if(th&&sub)th=em.indexOf(String(sub))!==-1;chk(th,"Expected function to throw"+(sub?" matching "+sub:""));},toViolate:function(sub){var r=typeof act==="function"?act():act,v=r&&r.violation?String(r.violation):"";var ok=Boolean(r&&r.ok===false&&v.length>0&&__vMatch(v,sub));chk(ok,"Expected sandbox violation "+(sub||"*")+" but got "+(v||"ok:true"));},toStayWithinFuel:function(maxF){var r=typeof act==="function"?act():act;var ok=Boolean(r&&r.ok===true&&typeof r.fuelUsed==="number"&&r.fuelUsed<=Number(maxF));chk(ok,"Expected fuelUsed <= "+maxF+" but got "+(r&&r.fuelUsed));},toStayWithinMemory:function(maxB){var r=typeof act==="function"?act():act;var ok=Boolean(r&&r.ok===true&&typeof r.memoryUsedBytes==="number"&&r.memoryUsedBytes<=Number(maxB));chk(ok,"Expected memoryUsedBytes <= "+maxB+" but got "+(r&&r.memoryUsedBytes));}};}var m={describe:function(n,fn){__st.suites.push(String(n));__st.be.push([]);__st.ae.push([]);try{fn();}finally{__st.suites.pop();__st.be.pop();__st.ae.pop();}},beforeEach:function(fn){__st.be[__st.be.length-1].push(fn);},afterEach:function(fn){__st.ae[__st.ae.length-1].push(fn);},test:function(n,fn,opts){var suite=__st.suites.join(" > "),full=suite?(suite+" > "+n):String(n),flt=globalThis.__slice_test_filter||"";if(flt&&full.indexOf(flt)===-1){__st.skipped++;return;}var iso=!(opts&&opts.isolateState===false),snap=iso?__slice_checkpoint():0,t0=performance.now(),err=null;try{for(var i=0;i<__st.be.length;i++)for(var j=0;j<__st.be[i].length;j++)__st.be[i][j]();var r=fn();if(r&&typeof r.then==="function"){r.then(function(){},function(e){err=String(e&&e.message||e);});}for(var k=__st.ae.length-1;k>=0;k--)for(var l=0;l<__st.ae[k].length;l++)__st.ae[k][l]();}catch(e){err=String(e&&e.message||e);}finally{if(snap>0){__slice_restore(snap);__st.rollbacks++;}}var dur=Math.max(1,Math.round((performance.now()-t0)*1000));if(!err){__st.passed++;__st.res.push({suite:suite,name:String(n),fullName:full,passed:true,durationUs:dur,checkpointId:snap,restored:snap>0,error:null});}else{__st.failed++;__st.res.push({suite:suite,name:String(n),fullName:full,passed:false,durationUs:dur,checkpointId:snap,restored:snap>0,error:err});}},expect:function(v){return __mkExp(v,false);},summary:function(){return{total:__st.passed+__st.failed+__st.skipped,passed:__st.passed,failed:__st.failed,skipped:__st.skipped,checkpointsRestored:__st.rollbacks,results:__st.res.slice()};},reset:function(){__st.suites=[];__st.be=[[]];__st.ae=[[]];__st.res=[];__st.passed=0;__st.failed=0;__st.skipped=0;__st.rollbacks=0;}};m.it=m.test;m.default=m;m.__esModule=true;__slice_builtins["slice:test"]=__slice_builtins["node:test"]=__slice_builtins["bun:test"]=m;"#,
                _ => return Ok(BoaVal::undefined()),
            };
            context.eval(boa_engine::Source::from_bytes(snippet))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_path_join"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let mut out = String::with_capacity(64);
            for (idx, arg) in args.iter().enumerate() {
                let s = arg.to_string(context)?.to_std_string_escaped();
                if s.is_empty() {
                    continue;
                }
                let normalized = if s.contains('\\') {
                    s.replace('\\', "/")
                } else {
                    s
                };
                let mut seg = normalized.as_str();
                if !out.is_empty() {
                    seg = seg.trim_start_matches('/');
                }
                if idx + 1 < args.len() && seg.len() > 1 {
                    seg = seg.trim_end_matches('/');
                }
                if seg.is_empty() {
                    continue;
                }
                if !out.is_empty() && !out.ends_with('/') {
                    out.push('/');
                }
                out.push_str(seg);
            }
            if out.is_empty() {
                out.push('.');
            }
            Ok(BoaVal::from(js_string!(out)))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fs_read_file_sync"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let path = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            if !check_boa_fs_read_ocap(&path) {
                return Ok(BoaVal::from(js_string!("")));
            }
            let norm = norm_vfs_path(&path);
            let content = BOA_VFS_CACHE
                .with(|c| c.borrow().get(&norm).cloned())
                .unwrap_or_else(|| std::fs::read_to_string(&path).unwrap_or_default());
            Ok(BoaVal::from(js_string!(content)))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fs_write_file_sync"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let path = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            if !check_boa_fs_write_ocap(&path) {
                return Ok(BoaVal::undefined());
            }
            let data = args
                .get(1)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let norm = norm_vfs_path(&path);
            if let Some(idx) = norm.rfind('/') {
                let parent = &norm[..idx];
                BOA_VFS_DIRS.with(|d| {
                    d.borrow_mut().insert(parent.to_string());
                });
            }
            BOA_VFS_CACHE.with(|c| {
                c.borrow_mut().insert(norm, data);
            });
            Ok(BoaVal::undefined())
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fs_append_file_sync"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let path = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            if !check_boa_fs_write_ocap(&path) {
                return Ok(BoaVal::undefined());
            }
            let data = args
                .get(1)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let norm = norm_vfs_path(&path);
            BOA_VFS_CACHE.with(|c| {
                let mut map = c.borrow_mut();
                let entry = map
                    .entry(norm)
                    .or_insert_with(|| std::fs::read_to_string(&path).unwrap_or_default());
                entry.push_str(&data);
            });
            Ok(BoaVal::undefined())
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fs_exists_sync"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let path = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let norm = norm_vfs_path(&path);
            let exists = BOA_VFS_CACHE.with(|c| c.borrow().contains_key(&norm))
                || BOA_VFS_DIRS.with(|d| d.borrow().contains(&norm))
                || std::path::Path::new(&path).exists();
            Ok(BoaVal::from(exists))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fs_mkdir_sync"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let path = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let norm = norm_vfs_path(&path);
            BOA_VFS_DIRS.with(|d| {
                d.borrow_mut().insert(norm);
            });
            Ok(BoaVal::undefined())
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fs_readdir_sync"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let path = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let norm_dir = norm_vfs_path(&path);
            let prefix = format!("{}/", norm_dir);
            let mut entries_set = std::collections::BTreeSet::new();
            BOA_VFS_CACHE.with(|c| {
                for k in c.borrow().keys() {
                    if let Some(rest) = k.strip_prefix(&prefix) {
                        if !rest.contains('/') && !rest.is_empty() {
                            entries_set.insert(rest.to_string());
                        }
                    }
                }
            });
            if let Ok(rd) = std::fs::read_dir(&path) {
                for entry in rd.flatten() {
                    entries_set.insert(entry.file_name().to_string_lossy().to_string());
                }
            }
            let entries: Vec<String> = entries_set.into_iter().collect();
            let json = serde_json::to_string(&entries).unwrap_or_else(|_| "[]".to_string());
            Ok(BoaVal::from(js_string!(json)))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fs_stat_sync"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let path = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let norm = norm_vfs_path(&path);
            let size = BOA_VFS_CACHE
                .with(|c| c.borrow().get(&norm).map(|v| v.len() as u64))
                .unwrap_or_else(|| std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0));
            Ok(BoaVal::from(size as f64))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fs_rm_sync"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let path = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let norm = norm_vfs_path(&path);
            let prefix = format!("{}/", norm);
            BOA_VFS_CACHE.with(|c| {
                let mut map = c.borrow_mut();
                map.remove(&norm);
                map.retain(|k, _| !k.starts_with(&prefix));
            });
            BOA_VFS_DIRS.with(|d| {
                let mut dirs = d.borrow_mut();
                dirs.remove(&norm);
                dirs.retain(|k| !k.starts_with(&prefix));
            });
            let p = std::path::Path::new(&path);
            if p.exists() {
                if p.is_dir() {
                    let _ = std::fs::remove_dir_all(p);
                } else {
                    let _ = std::fs::remove_file(p);
                }
            }
            Ok(BoaVal::undefined())
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_os_tmpdir"),
        0,
        NativeFunction::from_fn_ptr(|_this, _args, _context| {
            let tmp = std::env::temp_dir()
                .to_string_lossy()
                .replace('\\', "/");
            Ok(BoaVal::from(js_string!(tmp)))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_crypto_sha256"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let input = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let hex = sha256_hex(input.as_bytes());
            Ok(BoaVal::from(js_string!(hex)))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_crypto_hmac_sha256"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let key = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let msg = args
                .get(1)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let hex = hmac_sha256_hex(key.as_bytes(), msg.as_bytes());
            Ok(BoaVal::from(js_string!(hex)))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_base64_encode"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let input = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let url_safe = args.get(1).and_then(|v| v.as_boolean()).unwrap_or(false);
            let encoded = base64_encode(input.as_bytes(), url_safe);
            Ok(BoaVal::from(js_string!(encoded)))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_base64_decode"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let input = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let decoded = String::from_utf8_lossy(&base64_decode(&input)).into_owned();
            Ok(BoaVal::from(js_string!(decoded)))
        }),
    );

    // Phase 7.3: Native Rust SemVer Fast-Path Intrinsics
    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_semver_valid"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let Some(first) = args.first() else {
                return Ok(BoaVal::null());
            };
            if first.is_null() || first.is_undefined() {
                return Ok(BoaVal::null());
            }
            let s = if let Some(obj) = first.as_object() {
                if let Ok(v) = obj.get(js_string!("version"), context) {
                    v.to_string(context)?.to_std_string_escaped()
                } else {
                    first.to_string(context)?.to_std_string_escaped()
                }
            } else {
                first.to_string(context)?.to_std_string_escaped()
            };
            match semver_parse_strict_xyz(&s) {
                Some((maj, min, pat)) => Ok(BoaVal::from(js_string!(format!("{}.{}.{}", maj, min, pat)))),
                None => Ok(BoaVal::null()),
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_semver_coerce"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let Some(first) = args.first() else {
                return Ok(BoaVal::null());
            };
            let s = first.to_string(context)?.to_std_string_escaped();
            match semver_coerce_xyz(&s) {
                Some((maj, min, pat)) => Ok(BoaVal::from(js_string!(format!("{}.{}.{}", maj, min, pat)))),
                None => Ok(BoaVal::null()),
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_semver_gt"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let a = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let b = args
                .get(1)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            if let (Some(va), Some(vb)) = (semver_parse_strict_xyz(&a), semver_parse_strict_xyz(&b)) {
                Ok(BoaVal::from(va > vb))
            } else {
                Ok(BoaVal::from(false))
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_semver_satisfies"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let ver = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let rng = args
                .get(1)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            match semver_satisfies_xyz(&ver, &rng) {
                Some(true) => Ok(BoaVal::from(1)),
                Some(false) => Ok(BoaVal::from(0)),
                None => Ok(BoaVal::from(-1)),
            }
        }),
    );

    // Phase 9B Native CJS Fast-Path Intrinsics (`ms`, `bytes`, `cookie`)
    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fast_ms"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, _context| {
            let Some(first) = args.first() else {
                return Ok(BoaVal::undefined());
            };
            if let Some(s) = first.as_string() {
                let raw = s.to_std_string_escaped();
                if let Some(ms_val) = fast_parse_ms(&raw) {
                    return Ok(BoaVal::from(ms_val));
                }
            } else if let Some(n) = first.as_number() {
                let long = args.get(1).and_then(|v| v.as_boolean()).unwrap_or(false);
                if let Some(formatted) = fast_format_ms(n, long) {
                    return Ok(BoaVal::from(js_string!(formatted)));
                }
            }
            Ok(BoaVal::undefined())
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fast_bytes_parse"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, _context| {
            let Some(first) = args.first() else {
                return Ok(BoaVal::undefined());
            };
            if let Some(n) = first.as_number() {
                return Ok(BoaVal::from(n));
            }
            if let Some(s) = first.as_string() {
                let raw = s.to_std_string_escaped();
                if let Some(b_val) = fast_parse_bytes(&raw) {
                    return Ok(BoaVal::from(b_val));
                }
            }
            Ok(BoaVal::undefined())
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fast_bytes_format"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, _context| {
            if args.get(1).is_some_and(|v| !v.is_undefined() && !v.is_null()) {
                return Ok(BoaVal::undefined());
            }
            let Some(n) = args.first().and_then(|v| v.as_number()) else {
                return Ok(BoaVal::undefined());
            };
            if let Some(formatted) = fast_format_bytes(n) {
                return Ok(BoaVal::from(js_string!(formatted)));
            }
            Ok(BoaVal::undefined())
        }),
    );

    // Phase N1.3 + F1: Zero-Copy WinterTC HTTP Request / Response & waitUntil Host Intrinsics
    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_http_is_active"),
        0,
        NativeFunction::from_fn_ptr(|_this, _args, _context| {
            let active = BOA_HTTP_REQ.with(|r| r.borrow().active);
            Ok(BoaVal::from(active))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_http_headers_all"),
        0,
        NativeFunction::from_fn_ptr(|_this, _args, _context| {
            let json = BOA_HTTP_REQ.with(|r| {
                let borrow = r.borrow();
                serde_json::to_string(&borrow.headers[..borrow.headers_len])
                    .unwrap_or_else(|_| "[]".to_string())
            });
            Ok(BoaVal::from(js_string!(json)))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_http_method"),
        0,
        NativeFunction::from_fn_ptr(|_this, _args, _context| {
            let s = BOA_HTTP_REQ.with(|r| r.borrow().method.clone());
            Ok(BoaVal::from(js_string!(s)))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_http_path"),
        0,
        NativeFunction::from_fn_ptr(|_this, _args, _context| {
            let s = BOA_HTTP_REQ.with(|r| r.borrow().path.clone());
            Ok(BoaVal::from(js_string!(s)))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_http_query"),
        0,
        NativeFunction::from_fn_ptr(|_this, _args, _context| {
            let s = BOA_HTTP_REQ.with(|r| r.borrow().query.clone());
            Ok(BoaVal::from(js_string!(s)))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_http_tenant"),
        0,
        NativeFunction::from_fn_ptr(|_this, _args, _context| {
            let s = BOA_HTTP_REQ.with(|r| r.borrow().tenant_id.clone());
            Ok(BoaVal::from(js_string!(s)))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_http_body"),
        0,
        NativeFunction::from_fn_ptr(|_this, _args, _context| {
            let s = BOA_HTTP_REQ.with(|r| r.borrow().body.clone());
            Ok(BoaVal::from(js_string!(s)))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_http_header"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let key = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let found = BOA_HTTP_REQ.with(|r| {
                let borrow = r.borrow();
                borrow.headers[..borrow.headers_len]
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case(&key))
                    .map(|(_, v)| v.clone())
            });
            match found {
                Some(v) => Ok(BoaVal::from(js_string!(v))),
                None => Ok(BoaVal::null()),
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_http_respond"),
        3,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let status = args
                .first()
                .and_then(|v| v.as_number())
                .map(|n| n as u16)
                .unwrap_or(200);
            let body = args
                .get(1)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let content_type = args
                .get(2)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_else(|| "application/json".to_string());
            BOA_HTTP_RES.with(|res| {
                *res.borrow_mut() = Some((status, body, content_type));
            });
            Ok(BoaVal::undefined())
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_mark_wait_until"),
        0,
        NativeFunction::from_fn_ptr(|_this, _args, _context| {
            BOA_WAIT_UNTIL_COUNT.with(|c| c.set(c.get() + 1));
            Ok(BoaVal::undefined())
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_kv_get"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let key = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let val = BOA_ACTIVE_TENANT.with(|t| edge_kv_get(&t.borrow(), &key));
            match val {
                Some(s) => Ok(BoaVal::from(js_string!(s))),
                None => Ok(BoaVal::null()),
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_kv_set"),
        3,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let key = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let val = args
                .get(1)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let ttl_ms = args
                .get(2)
                .and_then(|v| v.as_number())
                .map(|n| n.max(0.0) as u64)
                .unwrap_or(0);
            let added = key.len().saturating_add(val.len());
            let allowed = BOA_KV_MEM_BUDGET.with(|m| {
                let (used, limit) = m.get();
                let next = used.saturating_add(added);
                if next > limit {
                    false
                } else {
                    m.set((next, limit));
                    true
                }
            });
            if !allowed {
                let tenant = BOA_ACTIVE_TENANT.with(|t| t.borrow().clone());
                let limit = BOA_KV_MEM_BUDGET.with(|m| m.get().1);
                BOA_VIOLATION.with(|v| {
                    *v.borrow_mut() = Some(SandboxViolation::MemoryCeilingExceeded {
                        tenant_id: tenant,
                        requested: added,
                        limit,
                    });
                });
                return Ok(BoaVal::from(false));
            }
            BOA_ACTIVE_TENANT.with(|t| edge_kv_set(&t.borrow(), &key, &val, ttl_ms));
            Ok(BoaVal::from(true))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_kv_del"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let key = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let removed = BOA_ACTIVE_TENANT.with(|t| edge_kv_del(&t.borrow(), &key));
            Ok(BoaVal::from(removed))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_kv_incr"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let key = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let delta = args
                .get(1)
                .and_then(|v| v.as_number())
                .map(|n| n as i64)
                .unwrap_or(1);
            let next = BOA_ACTIVE_TENANT.with(|t| edge_kv_incr(&t.borrow(), &key, delta));
            Ok(BoaVal::from(next as f64))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_det_random"),
        0,
        NativeFunction::from_fn_ptr(|_this, _args, _context| {
            let val = BOA_DET_STATE.with(|s| {
                if let Some((mut prng, clk)) = s.get() {
                    let r = splitmix64_next_f64(&mut prng);
                    s.set(Some((prng, clk)));
                    r
                } else {
                    -1.0
                }
            });
            Ok(BoaVal::from(val))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_det_now"),
        0,
        NativeFunction::from_fn_ptr(|_this, _args, _context| {
            let val = BOA_DET_STATE.with(|s| {
                if let Some((prng, clk)) = s.get() {
                    let next_clk = clk.wrapping_add(1);
                    s.set(Some((prng, next_clk)));
                    next_clk as f64
                } else {
                    -1.0
                }
            });
            Ok(BoaVal::from(val))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_checkpoint"),
        0,
        NativeFunction::from_fn_ptr(|_this, _args, _context| {
            let id = BOA_ACTIVE_TENANT.with(|t| checkpoint_tenant_state(&t.borrow()));
            Ok(BoaVal::from(id as f64))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_restore"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, _context| {
            let id = args
                .first()
                .and_then(|v| v.as_number())
                .map(|n| n as u64)
                .unwrap_or(0);
            let ok = BOA_ACTIVE_TENANT.with(|t| restore_tenant_state(&t.borrow(), id));
            Ok(BoaVal::from(ok))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_eval_sandbox"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let code = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let opts_json = args
                .get(1)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_else(|| "{}".to_string());
            let opts: SandboxEvalOptions = serde_json::from_str(&opts_json).unwrap_or_default();

            // Save parent's thread-local execution state before entering nested child SliceIsolate
            let saved_stdout = BOA_STDOUT.with(|out| std::mem::take(&mut *out.borrow_mut()));
            let saved_violation = BOA_VIOLATION.with(|v| v.borrow_mut().take());
            let saved_tenant = BOA_ACTIVE_TENANT.with(|t| t.borrow().clone());
            let saved_caps = BOA_ACTIVE_CAPS.with(|c| c.borrow_mut().take());
            let saved_net_hosts = BOA_NET_HOSTS.with(|h| std::mem::take(&mut *h.borrow_mut()));
            let saved_egress_secrets =
                BOA_EGRESS_SECRETS.with(|s| std::mem::take(&mut *s.borrow_mut()));
            let saved_fetch_mocks = BOA_FETCH_MOCKS.with(|m| m.borrow().clone());
            let saved_kv_budget = BOA_KV_MEM_BUDGET.with(|m| m.get());
            let saved_det = BOA_DET_STATE.with(|s| s.get());

            let mut parent_caps = CapabilityToken::pure_compute();
            if let Some((_, ref r_prefs, ref w_prefs)) = saved_caps {
                for r in r_prefs {
                    parent_caps = parent_caps.with_read_prefix(r.clone());
                }
                for w in w_prefs {
                    parent_caps = parent_caps.with_write_prefix(w.clone());
                }
            }
            for nh in &saved_net_hosts {
                parent_caps = parent_caps.with_net_host(nh.clone());
            }
            let parent_rem_mem = saved_kv_budget.1.saturating_sub(saved_kv_budget.0);
            let mut proxy_parent = SliceIsolate::spawn(
                saved_tenant.clone(),
                98,
                parent_caps,
                5_000_000,
                parent_rem_mem.max(65536),
            );
            for sec in &saved_egress_secrets {
                proxy_parent.add_egress_audit_secret(sec.clone());
            }
            let outcome = proxy_parent.eval_sandbox(&code, opts);

            // Restore parent's thread-local execution state
            BOA_STDOUT.with(|out| *out.borrow_mut() = saved_stdout);
            BOA_VIOLATION.with(|v| *v.borrow_mut() = saved_violation);
            BOA_ACTIVE_TENANT.with(|t| *t.borrow_mut() = saved_tenant);
            BOA_ACTIVE_CAPS.with(|c| *c.borrow_mut() = saved_caps);
            BOA_NET_HOSTS.with(|h| *h.borrow_mut() = saved_net_hosts);
            BOA_EGRESS_SECRETS.with(|s| *s.borrow_mut() = saved_egress_secrets);
            BOA_FETCH_MOCKS.with(|m| *m.borrow_mut() = saved_fetch_mocks);
            BOA_KV_MEM_BUDGET.with(|m| m.set(saved_kv_budget));
            BOA_DET_STATE.with(|s| s.set(saved_det));

            let json = serde_json::to_string(&outcome).unwrap_or_else(|_| "{}".to_string());
            Ok(BoaVal::from(js_string!(json)))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_mock_fetch"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let key = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let payload = args
                .get(1)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            if !key.is_empty() && !payload.is_empty() {
                BOA_FETCH_MOCKS.with(|m| {
                    m.borrow_mut().insert(key, payload);
                });
            }
            Ok(BoaVal::undefined())
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_env_get"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let key = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let found = BOA_ENV_MAP.with(|m| m.borrow().get(&key).cloned());
            match found {
                Some(v) => Ok(BoaVal::from(js_string!(v))),
                None => {
                    let tenant = BOA_ACTIVE_TENANT.with(|t| t.borrow().clone());
                    BOA_VIOLATION.with(|v| {
                        *v.borrow_mut() = Some(SandboxViolation::EnvAccessDenied {
                            tenant_id: tenant,
                            key,
                        });
                    });
                    Ok(BoaVal::from(js_string!("")))
                }
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_env_json"),
        0,
        NativeFunction::from_fn_ptr(|_this, _args, _context| {
            let json = BOA_ENV_MAP
                .with(|m| serde_json::to_string(&*m.borrow()).unwrap_or_else(|_| "{}".to_string()));
            Ok(BoaVal::from(js_string!(json)))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fetch_sync"),
        4,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let url = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let method = args
                .get(1)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped().to_ascii_uppercase())
                .unwrap_or_else(|| "GET".to_string());
            let headers_json = args
                .get(2)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_else(|| "[]".to_string());
            let body = args
                .get(3)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();

            let Some(parts) = parse_outbound_http_url(&url) else {
                let err_json = serde_json::json!({ "__error": format!("Invalid outbound URL: {}", url) }).to_string();
                return Ok(BoaVal::from(js_string!(err_json)));
            };

            if let Err(err_msg) =
                check_boa_outbound_fetch_ocap_and_dlp(&parts, &url, &headers_json, &body)
            {
                let err_json = serde_json::json!({ "__error": err_msg }).to_string();
                return Ok(BoaVal::from(js_string!(err_json)));
            }

            // 1. Check in-memory mock cassette (`Slice.mockFetch(...)`)
            let method_url_key = format!("{} {}", method, url);
            if let Some(mock_json) = BOA_FETCH_MOCKS.with(|m| {
                let map = m.borrow();
                map.get(&method_url_key)
                    .or_else(|| map.get(&url))
                    .cloned()
            }) {
                return Ok(BoaVal::from(js_string!(mock_json)));
            }

            // 2. Check deterministic KV cassette (`fetch_cassette:<METHOD>:<URL>`)
            let cassette_key = format!("fetch_cassette:{}:{}", method, url);
            let tenant_id = BOA_ACTIVE_TENANT.with(|t| t.borrow().clone());
            if let Some(cassette_json) = edge_kv_get(&tenant_id, &cassette_key) {
                return Ok(BoaVal::from(js_string!(cassette_json)));
            }

            // 3. Execute real outbound HTTP/1.1 request over TCP
            let req_headers: Vec<(String, String)> =
                serde_json::from_str(&headers_json).unwrap_or_default();
            match execute_outbound_http11(&parts, &method, &req_headers, &body) {
                Ok(resp_json) => {
                    if BOA_DET_STATE.with(|s| s.get().is_some()) {
                        edge_kv_set(&tenant_id, &cassette_key, &resp_json, 0);
                    }
                    Ok(BoaVal::from(js_string!(resp_json)))
                }
                Err(err_msg) => {
                    let err_json = serde_json::json!({ "__error": err_msg }).to_string();
                    Ok(BoaVal::from(js_string!(err_json)))
                }
            }
        }),
    );
}

