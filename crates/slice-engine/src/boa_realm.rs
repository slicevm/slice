//! Warm `boa_engine` Realm Pool, Thread-Local Compartment State, `CORE_PRELUDE` WinterTC/SSE/Fetch Bootstrap,
//! and Native Host Intrinsics (`node:*` & `slice:test` lazy loader).

use crate::event_loop::{
    cancel_timer, clear_event_loop_state, drive_event_loop_to_completion, enqueue_async_fetch,
    restore_event_loop_state, save_and_reset_event_loop_state, schedule_timer,
};
use crate::intrinsics::{
    base64_decode, base64_encode, bytes_to_hex, constant_time_eq, fast_format_bytes,
    fast_format_ms, fast_parse_bytes, fast_parse_ms, fill_random_bytes, generate_uuid_v4,
    hmac_sha1_bytes, hmac_sha256_bytes, hmac_sha256_hex, hmac_sha384_bytes, hmac_sha512_bytes,
    semver_coerce_xyz, semver_parse_strict_xyz, semver_satisfies_xyz, sha1_bytes, sha256_bytes,
    sha256_hex, sha384_bytes, sha512_bytes, whatwg_atob, whatwg_btoa,
};
use crate::isolate::{SandboxEvalOptions, SliceIsolate};
use crate::outbound_http::{
    check_boa_outbound_fetch_ocap_and_dlp, execute_outbound_fetch_with_redirects,
    parse_outbound_http_url,
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
    pub(crate) ses_check_fn: Option<boa_engine::JsObject>,
}

pub(crate) type ActiveBoaCaps = (String, Vec<String>, Vec<String>);
pub(crate) type CapturedBoaHttpRes = (u16, String, String, Vec<(String, String)>);

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
    pub(crate) static BOA_HTTP_RES: std::cell::RefCell<Option<CapturedBoaHttpRes>> =
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

    const CORE_PRELUDE: &str = r##"globalThis.console={log:__slice_console_log,info:__slice_console_log,warn:__slice_console_log,error:__slice_console_log};globalThis.print=__slice_console_log;globalThis.env=function(k){return __slice_env_get(String(k));};globalThis.process={pid:1,env:{NODE_ENV:"production"},cwd:function(){return ".";},get platform(){return __slice_os_platform();},get arch(){return __slice_os_arch();},version:"v24.0.0",versions:{node:"24.0.0"},argv:["slice"]};globalThis.__slice_check_ses=function(){var p=[Object.prototype,Array.prototype,Function.prototype,String.prototype],n=["Object.prototype","Array.prototype","Function.prototype","String.prototype"];for(var i=0;i<4;i++){var ks=Object.keys(p[i]);if(ks.length>0){for(var j=0;j<ks.length;j++)delete p[i][ks[j]];return n[i];}}return "";};globalThis.__slice_sync_process_env=function(load){if(!load){globalThis.process.env={NODE_ENV:"production"};return;}var m=JSON.parse(__slice_env_json());m.NODE_ENV=m.NODE_ENV||"production";globalThis.process.env=m;};var __b2s=function(e){return e==="base64"?__slice_base64_encode(this._s,false):e==="base64url"?__slice_base64_encode(this._s,true):this._s;};globalThis.Buffer={from:function(i,e){var r=typeof i==="string"?i:String(i),s=(e==="base64"||e==="base64url")?__slice_base64_decode(r):r;return{_s:s,length:s.length,toString:__b2s};},isBuffer:function(){return false;},byteLength:function(s){return String(s).length;}};var __orand=Math.random,__onow=Date.now;Math.random=function(){var v=__slice_det_random();return v>=0?v:__orand();};Date.now=function(){var v=__slice_det_now();return v>=0?v:__onow();};globalThis.performance={now:function(){var v=__slice_det_now();return v>=0?(v-1700000000000):(__onow()-1700000000000);}};globalThis.setTimeout=function(cb,ms){var a=[];for(var i=2;i<arguments.length;i++)a.push(arguments[i]);return __slice_timer_set(cb,Number(ms)||0,false,a);};globalThis.clearTimeout=function(id){__slice_timer_clear(Number(id)||0);};globalThis.setInterval=function(cb,ms){var a=[];for(var i=2;i<arguments.length;i++)a.push(arguments[i]);return __slice_timer_set(cb,Number(ms)||0,true,a);};globalThis.clearInterval=globalThis.clearTimeout;globalThis.queueMicrotask=function(cb){if(typeof cb!=="function")throw new TypeError("queueMicrotask requires a callback function");Promise.resolve().then(cb);};function AbortSignal(){this.aborted=false;this.reason=undefined;this.onabort=null;this._ls=[];}AbortSignal.prototype.addEventListener=function(t,fn){if(t==="abort"&&typeof fn==="function")this._ls.push(fn);};AbortSignal.prototype.removeEventListener=function(t,fn){if(t==="abort"){var nx=[];for(var i=0;i<this._ls.length;i++)if(this._ls[i]!==fn)nx.push(this._ls[i]);this._ls=nx;}};AbortSignal.prototype.throwIfAborted=function(){if(this.aborted)throw this.reason;};AbortSignal.prototype._fire=function(r){if(this.aborted)return;this.aborted=true;this.reason=r!==undefined?r:new Error("AbortError: The operation was aborted");var ev={type:"abort",target:this};if(typeof this.onabort==="function")try{this.onabort(ev);}catch(_e){}var ls=this._ls.slice();this._ls=[];for(var i=0;i<ls.length;i++)try{ls[i](ev);}catch(_e2){}};AbortSignal.abort=function(r){var s=new AbortSignal();s._fire(r!==undefined?r:new Error("AbortError: The operation was aborted"));return s;};AbortSignal.timeout=function(ms){var s=new AbortSignal();setTimeout(function(){s._fire(new Error("TimeoutError: The operation was aborted due to timeout"));},Number(ms)||0);return s;};function AbortController(){this.signal=new AbortSignal();}AbortController.prototype.abort=function(r){this.signal._fire(r);};globalThis.AbortSignal=AbortSignal;globalThis.AbortController=AbortController;function __fmtSse(ev){if(ev===undefined||ev===null)return "";if(typeof ev==="string"){if(ev.indexOf("\n")===-1)return "data: "+ev+"\n\n";var ls=ev.split("\n"),o="";for(var i=0;i<ls.length;i++)o+="data: "+ls[i]+"\n";return o+"\n";}var s="";if(ev.id!==undefined&&ev.id!==null)s+="id: "+String(ev.id)+"\n";if(ev.event!==undefined&&ev.event!==null)s+="event: "+String(ev.event)+"\n";if(ev.retry!==undefined&&ev.retry!==null)s+="retry: "+Number(ev.retry)+"\n";var d=ev.data!==undefined?(typeof ev.data==="string"?ev.data:JSON.stringify(ev.data)):JSON.stringify(ev);if(d.indexOf("\n")===-1)return s+"data: "+d+"\n\n";var dl=d.split("\n");for(var j=0;j<dl.length;j++)s+="data: "+dl[j]+"\n";return s+"\n";}function __colSse(src){var out="";if(Array.isArray(src)){for(var i=0;i<src.length;i++)out+=__fmtSse(src[i]);}else if(typeof src==="function"){var r=src(function(ev){out+=__fmtSse(ev);});if(r&&typeof r.next==="function"){var st=r.next();while(!st.done){out+=__fmtSse(st.value);st=r.next();}}else if(Array.isArray(r)){for(var j=0;j<r.length;j++)out+=__fmtSse(r[j]);}}else if(src&&typeof src.next==="function"){var s2=src.next();while(!s2.done){out+=__fmtSse(s2.value);s2=src.next();}}else if(src!==undefined&&src!==null){out+=__fmtSse(src);}return out;}function ReadableStream(src){var chunks=[],closed=false;this._chunks=chunks;if(src&&typeof src.start==="function"){src.start({enqueue:function(c){if(!closed)chunks.push(typeof c==="string"?c:String(c));},close:function(){closed=true;}});}}ReadableStream.prototype._toText=function(){return this._chunks.join("");};ReadableStream.prototype.getReader=function(){var cs=this._chunks,idx=0;return{read:function(){if(idx<cs.length)return Promise.resolve({value:cs[idx++],done:false});return Promise.resolve({value:undefined,done:true});},releaseLock:function(){}};};globalThis.ReadableStream=ReadableStream;function __doFetch(input,init){var req=new Request(input,init),sig=(init&&init.signal)||req.signal||null;if(sig&&sig.aborted)throw(sig.reason||new Error("AbortError: The operation was aborted"));var raw=JSON.parse(__slice_fetch_sync(req.url,req.method,JSON.stringify(req.headers.entries()),req._body,req.redirect||"follow"));if(raw.__error)throw new Error(raw.__error);var r=new Response(raw.body,{status:raw.status,statusText:raw.statusText,headers:raw.headers});r.url=raw.finalUrl||req.url;r.redirected=Boolean(raw.redirected);return r;}globalThis.fetch=function(input,init){try{var req=new Request(input,init),sig=(init&&init.signal)||req.signal||null;if(sig&&sig.aborted)return Promise.reject(sig.reason||new Error("AbortError: The operation was aborted"));var hdrs=JSON.stringify(req.headers.entries()),fast=__slice_fetch_try_fast(req.url,req.method,hdrs,req._body,req.redirect||"follow");if(fast!==""){var raw=JSON.parse(fast);if(raw.__error)return Promise.reject(new Error(raw.__error));var r=new Response(raw.body,{status:raw.status,statusText:raw.statusText,headers:raw.headers});r.url=raw.finalUrl||req.url;r.redirected=Boolean(raw.redirected);return Promise.resolve(r);}return new Promise(function(res,rej){var settled=false;if(sig&&typeof sig.addEventListener==="function"){sig.addEventListener("abort",function(){if(!settled){settled=true;rej(sig.reason||new Error("AbortError: The operation was aborted"));}});}__slice_fetch_enqueue(req.url,req.method,hdrs,req._body,req.redirect||"follow",function(rawStr){if(settled)return;settled=true;var raw2=JSON.parse(rawStr);if(raw2.__error){rej(new Error(raw2.__error));return;}var r2=new Response(raw2.body,{status:raw2.status,statusText:raw2.statusText,headers:raw2.headers});r2.url=raw2.finalUrl||req.url;r2.redirected=Boolean(raw2.redirected);res(r2);},function(errReason){if(settled)return;settled=true;rej(errReason instanceof Error?errReason:new Error(String(errReason)));},sig);});}catch(e){return Promise.reject(e);}};function __tMod(){return __slice_builtins["slice:test"]||__slice_load_builtin("slice:test");}globalThis.describe=function(n,f){return __tMod().describe(n,f);};globalThis.test=globalThis.it=function(n,f,o){return __tMod().test(n,f,o);};globalThis.beforeEach=function(f){return __tMod().beforeEach(f);};globalThis.afterEach=function(f){return __tMod().afterEach(f);};globalThis.expect=function(v){return __tMod().expect(v);};globalThis.__slice_wu=[];globalThis.Slice={request:function(){var t=__slice_http_tenant();return{method:__slice_http_method(),path:__slice_http_path(),query:__slice_http_query(),tenant:t,tenantId:t,body:__slice_http_body(),header:__slice_http_header};},respond:function(s,b,ct){return __slice_http_respond(s,typeof b==="string"?b:JSON.stringify(b),ct||"application/json","");},streamSSE:function(src,init){var r=Response.sse(src,init);__slice_http_respond(r.status,r._body,r._ct,"");return r;},fetchSync:__doFetch,mockFetch:function(a,b,c){var m="*",u=String(a),o=b||{};if(c!==undefined){m=String(a).toUpperCase();u=String(b);o=c||{};}var h=new Headers(o.headers),bd=o.body===undefined||o.body===null?"":(typeof o.body==="string"?o.body:JSON.stringify(o.body));if(typeof o.body==="object"&&o.body!==null&&!h._m["content-type"])h._m["content-type"]="application/json";__slice_mock_fetch(m==="*"?u:(m+" "+u),JSON.stringify({status:o.status||200,statusText:o.statusText||"OK",headers:h.entries(),body:bd}));},waitUntil:function(f){__slice_wu.push(f);__slice_mark_wait_until();},kv:{get:function(k){return __slice_kv_get(String(k));},set:function(k,v,ttl){return __slice_kv_set(String(k),typeof v==="string"?v:JSON.stringify(v),ttl||0);},del:function(k){return __slice_kv_del(String(k));},incr:function(k,d){return __slice_kv_incr(String(k),d===undefined?1:Number(d));}},checkpoint:function(){return __slice_checkpoint();},restore:function(id){return __slice_restore(Number(id));},evalSandbox:function(code,opts){return JSON.parse(__slice_eval_sandbox(String(code),opts?JSON.stringify(opts):"{}"));},testSummary:function(){return __tMod().summary();},resetTests:function(){return __tMod().reset();}};function __uspD(s){if(s.indexOf("+")===-1&&s.indexOf("%")===-1)return s;var r=s.indexOf("+")===-1?s:s.split("+").join(" ");if(r.indexOf("%")===-1)return r;try{return decodeURIComponent(r);}catch(_e){return r;}}function __uspE(s){var str=String(s),e=encodeURIComponent(str);return str.indexOf(" ")===-1?e:e.split("%20").join("+");}function URLSearchParams(init,u){this._p=[];this._u=u||null;if(!init)return;if(typeof init==="string"){var q=init.charCodeAt(0)===63?init.slice(1):init;if(q.length>0){var ps=q.split("&");for(var i=0;i<ps.length;i++){var sg=ps[i];if(!sg)continue;var eq=sg.indexOf("=");if(eq===-1)this._p.push([__uspD(sg),""]);else this._p.push([__uspD(sg.slice(0,eq)),__uspD(sg.slice(eq+1))]);}}}else if(Array.isArray(init)){for(var j=0;j<init.length;j++){var pr=init[j];if(pr&&pr.length>=2)this._p.push([String(pr[0]),String(pr[1])]);}}else if(typeof init==="object"){if(Array.isArray(init._p)){for(var m=0;m<init._p.length;m++)this._p.push([init._p[m][0],init._p[m][1]]);}else{var ks=Object.keys(init);for(var k=0;k<ks.length;k++)this._p.push([ks[k],String(init[ks[k]])]);}}}URLSearchParams.prototype._sync=function(){if(this._u)this._u._syncSearch();};URLSearchParams.prototype.get=function(n){var k=String(n);for(var i=0;i<this._p.length;i++)if(this._p[i][0]===k)return this._p[i][1];return null;};URLSearchParams.prototype.getAll=function(n){var k=String(n),r=[];for(var i=0;i<this._p.length;i++)if(this._p[i][0]===k)r.push(this._p[i][1]);return r;};URLSearchParams.prototype.has=function(n){var k=String(n);for(var i=0;i<this._p.length;i++)if(this._p[i][0]===k)return true;return false;};URLSearchParams.prototype.set=function(n,v){var k=String(n),s=String(v),p=this._p,len=p.length,fi=-1;for(var i=0;i<len;i++){if(p[i][0]===k){if(fi===-1){p[i][1]=s;fi=i;}else{p.splice(i,1);i--;len--;}}}if(fi===-1)p.push([k,s]);if(this._u)this._u._syncSearch();};URLSearchParams.prototype.append=function(n,v){this._p.push([String(n),String(v)]);if(this._u)this._u._syncSearch();};URLSearchParams.prototype.delete=function(n){var k=String(n),nx=[];for(var i=0;i<this._p.length;i++)if(this._p[i][0]!==k)nx.push(this._p[i]);this._p=nx;if(this._u)this._u._syncSearch();};URLSearchParams.prototype.sort=function(){this._p.sort(function(a,b){return a[0]<b[0]?-1:(a[0]>b[0]?1:0);});if(this._u)this._u._syncSearch();};URLSearchParams.prototype.forEach=function(cb,ta){for(var i=0;i<this._p.length;i++)cb.call(ta,this._p[i][1],this._p[i][0],this);};URLSearchParams.prototype.entries=function(){return this._p.slice();};URLSearchParams.prototype.keys=function(){var r=[];for(var i=0;i<this._p.length;i++)r.push(this._p[i][0]);return r;};URLSearchParams.prototype.values=function(){var r=[];for(var i=0;i<this._p.length;i++)r.push(this._p[i][1]);return r;};URLSearchParams.prototype.toString=function(){var len=this._p.length;if(len===0)return "";var s="";for(var i=0;i<len;i++){if(i>0)s+="&";s+=__uspE(this._p[i][0])+"="+__uspE(this._p[i][1]);}return s;};if(typeof Symbol!=="undefined"&&Symbol.iterator)URLSearchParams.prototype[Symbol.iterator]=function(){return this._p[Symbol.iterator]();};Object.defineProperty(URLSearchParams.prototype,"size",{get:function(){return this._p.length;}});function __normP(p){if(!p||p==="/")return "/";if(p.indexOf("/.")===-1&&p.indexOf("//")===-1)return p;var segs=p.split("/"),out=[];for(var i=0;i<segs.length;i++){var s=segs[i];if(!s||s===".")continue;if(s==="..")out.pop();else out.push(s);}return "/"+out.join("/")+(p.length>1&&p.charCodeAt(p.length-1)===47&&out.length>0?"/":"");}function URL(input,base){var raw=input instanceof URL?input.href:String(input).trim();if(base!==undefined&&raw.indexOf("https://")!==0&&raw.indexOf("http://")!==0&&raw.indexOf("://")===-1){var b=base instanceof URL?base:new URL(String(base));if(raw.indexOf("//")===0)raw=b.protocol+raw;else if(raw.charCodeAt(0)===47)raw=b.origin+raw;else if(raw.charCodeAt(0)===63)raw=b.origin+b.pathname+raw;else if(raw.charCodeAt(0)===35)raw=b.origin+b.pathname+b.search+raw;else{var dir=b.pathname.slice(0,b.pathname.lastIndexOf("/")+1)||"/";raw=b.origin+dir+raw;}}var hi=raw.indexOf("#"),hash="";if(hi!==-1){hash=raw.slice(hi);raw=raw.slice(0,hi);}var qi=raw.indexOf("?"),search="";if(qi!==-1){search=raw.slice(qi);raw=raw.slice(0,qi);}var si=raw.indexOf("://");if(si===-1)throw new TypeError("Invalid URL: "+input);this.protocol=raw.slice(0,si+1).toLowerCase();var rest=raw.slice(si+3),pi=rest.indexOf("/"),authHost=pi===-1?rest:rest.slice(0,pi),pathRaw=pi===-1?"/":rest.slice(pi);var ai=authHost.indexOf("@"),hostPort=authHost;this.username="";this.password="";if(ai!==-1){var up=authHost.slice(0,ai),ci=up.indexOf(":");this.username=ci===-1?up:up.slice(0,ci);this.password=ci===-1?"":up.slice(ci+1);hostPort=authHost.slice(ai+1);}var pci=hostPort.lastIndexOf(":");if(pci!==-1&&hostPort.indexOf("]")<pci){this.hostname=hostPort.slice(0,pci).toLowerCase();var prt=hostPort.slice(pci+1);if((this.protocol==="http:"&&prt==="80")||(this.protocol==="https:"&&prt==="443"))prt="";this.port=prt;}else{this.hostname=hostPort.toLowerCase();this.port="";}this.host=this.port?this.hostname+":"+this.port:this.hostname;this.origin=this.protocol+"//"+this.host;this.pathname=__normP(pathRaw);this.hash=hash==="#"?"":hash;this.searchParams=new URLSearchParams(search,this);this._syncSearch();}URL.prototype._syncSearch=function(){if(this.searchParams._p.length===0){this.search="";this.href=this.origin+this.pathname+this.hash;return;}var qs=this.searchParams.toString();this.search=qs?"?"+qs:"";this.href=this.origin+this.pathname+this.search+this.hash;};URL.prototype.toString=URL.prototype.toJSON=function(){return this.href;};function Headers(init){if(init==="__SLICE_LIVE_REQ__"){this._live=true;this._m=null;return;}this._live=false;this._m=Object.create(null);if(!init)return;if(init instanceof Headers){if(init._m){var ik=Object.keys(init._m);for(var n=0;n<ik.length;n++)this._m[ik[n]]=init._m[ik[n]];}else{var es=init.entries();for(var i=0;i<es.length;i++)this._m[es[i][0]]=es[i][1];}}else if(Array.isArray(init)){for(var j=0;j<init.length;j++)if(init[j]&&init[j].length>=2)this.append(init[j][0],init[j][1]);}else if(typeof init==="object"){var ks=Object.keys(init);for(var k=0;k<ks.length;k++)if(init[ks[k]]!==undefined)this._m[ks[k].toLowerCase()]=String(init[ks[k]]);}}Headers.prototype._ensure=function(){if(this._live&&!this._m){this._m=Object.create(null);var raw=JSON.parse(__slice_http_headers_all());for(var i=0;i<raw.length;i++){var k=raw[i][0].toLowerCase();this._m[k]=this._m[k]?this._m[k]+", "+raw[i][1]:raw[i][1];}this._live=false;}};Headers.prototype.get=function(n){var k=String(n).toLowerCase();if(this._m&&k in this._m)return this._m[k];if(this._live)return __slice_http_header(k);return null;};Headers.prototype.has=function(n){return this.get(n)!==null;};Headers.prototype.set=function(n,v){this._ensure();this._m[String(n).toLowerCase()]=String(v);};Headers.prototype.append=function(n,v){this._ensure();var k=String(n).toLowerCase(),s=String(v);this._m[k]=k in this._m?this._m[k]+", "+s:s;};Headers.prototype.delete=function(n){this._ensure();delete this._m[String(n).toLowerCase()];};Headers.prototype.entries=function(){this._ensure();var ks=Object.keys(this._m).sort(),r=[];for(var i=0;i<ks.length;i++)r.push([ks[i],this._m[ks[i]]]);return r;};Headers.prototype.keys=function(){this._ensure();return Object.keys(this._m).sort();};Headers.prototype.values=function(){var es=this.entries(),r=[];for(var i=0;i<es.length;i++)r.push(es[i][1]);return r;};Headers.prototype.forEach=function(cb,ta){var es=this.entries();for(var i=0;i<es.length;i++)cb.call(ta,es[i][1],es[i][0],this);};if(typeof Symbol!=="undefined"&&Symbol.iterator)Headers.prototype[Symbol.iterator]=function(){return this.entries()[Symbol.iterator]();};function Request(input,init){if(input==="__SLICE_LIVE_REQ__"){this.method=__slice_http_method()||"GET";var h=__slice_http_header("host")||"localhost",p=__slice_http_path()||"/",q=__slice_http_query();this.url="http://"+h+p+(q?"?"+q:"");this.headers=new Headers("__SLICE_LIVE_REQ__");this._body=__slice_http_body()||"";this.redirect="follow";this.signal=null;this.bodyUsed=false;return;}var o=init||{},isReq=input instanceof Request;this.url=isReq?input.url:(input instanceof URL?input.href:String(input));this.method=o.method?String(o.method).toUpperCase():(isReq?input.method:"GET");this.headers=o.headers instanceof Headers?o.headers:new Headers(o.headers||(isReq?input.headers:undefined));this._body=o.body!==undefined&&o.body!==null?(typeof o.body==="string"?o.body:JSON.stringify(o.body)):(isReq?input._body:"");this.redirect=o.redirect?String(o.redirect):(isReq&&input.redirect?input.redirect:"follow");this.signal=o.signal||(isReq&&input.signal?input.signal:null);this.bodyUsed=false;}Request.prototype.text=function(){this.bodyUsed=true;return Promise.resolve(this._body);};Request.prototype.json=function(){this.bodyUsed=true;try{return Promise.resolve(JSON.parse(this._body));}catch(e){return Promise.reject(e);}};Request.prototype.textSync=function(){this.bodyUsed=true;return this._body;};Request.prototype.jsonSync=function(){this.bodyUsed=true;return JSON.parse(this._body);};Request.prototype.clone=function(){return new Request(this.url,{method:this.method,headers:new Headers(this.headers),body:this._body,redirect:this.redirect,signal:this.signal});};function Response(body,init){var o=init||{};this.status=o.status!==undefined?Number(o.status):200;this.statusText=o.statusText!==undefined?String(o.statusText):(this.status===200?"OK":"");this.ok=this.status>=200&&this.status<300;this.headers=o.headers instanceof Headers?o.headers:new Headers(o.headers);this._data=undefined;this._body=body===undefined||body===null?"":(body instanceof ReadableStream?body._toText():(typeof body==="string"?body:JSON.stringify(body)));this._ct=(this.headers._m&&this.headers._m["content-type"])||"text/plain;charset=UTF-8";this._isSliceRes=true;this.bodyUsed=false;}Response.prototype.text=function(){this.bodyUsed=true;if(this._data!==undefined&&!this._body)this._body=JSON.stringify(this._data);return Promise.resolve(this._body);};Response.prototype.json=function(){this.bodyUsed=true;if(this._data!==undefined)return Promise.resolve(this._data);try{return Promise.resolve(JSON.parse(this._body));}catch(e){return Promise.reject(e);}};Response.prototype.textSync=function(){this.bodyUsed=true;if(this._data!==undefined&&!this._body)this._body=JSON.stringify(this._data);return this._body;};Response.prototype.jsonSync=function(){this.bodyUsed=true;if(this._data!==undefined)return this._data;return JSON.parse(this._body);};Response.prototype.clone=function(){if(this._data!==undefined&&!this._body)this._body=JSON.stringify(this._data);return new Response(this._body,{status:this.status,statusText:this.statusText,headers:new Headers(this.headers)});};Response.json=function(data,init){var o=init?(typeof init==="number"?{status:init}:init):{},h=o.headers instanceof Headers?o.headers:new Headers(o.headers);if(!h._m["content-type"])h._m["content-type"]="application/json";var r=new Response(null,{status:o.status||200,statusText:o.statusText,headers:h});r._data=data;r._ct=h._m["content-type"];return r;};Response.sse=function(src,init){var o=init?(typeof init==="number"?{status:init}:init):{},h=o.headers instanceof Headers?o.headers:new Headers(o.headers);if(!h._m["content-type"])h._m["content-type"]="text/event-stream; charset=utf-8";if(!h._m["cache-control"])h._m["cache-control"]="no-cache";var r=new Response(__colSse(src),{status:o.status||200,statusText:o.statusText,headers:h});r._ct=h._m["content-type"];return r;};Response.redirect=function(url,status){var st=status||302;return new Response("",{status:st,headers:{location:String(url)}});};function __resHdrJson(h){if(!h||!h._m)return "";var ks=Object.keys(h._m),out=[];for(var i=0;i<ks.length;i++){var k=ks[i];if(k!=="content-type"&&k!=="content-length")out.push([k,h._m[k]]);}return out.length>0?JSON.stringify(out):"";}globalThis.URLSearchParams=URLSearchParams;globalThis.URL=URL;globalThis.Headers=Headers;globalThis.Request=Request;globalThis.Response=Response;var __slice_web_ctx={waitUntil:Slice.waitUntil,passThroughOnException:function(){}};globalThis.__slice_make_web_request=function(){return new Request("__SLICE_LIVE_REQ__");};globalThis.__slice_dispatch_fetch=function(h){if(!__slice_http_is_active())return;var fn=typeof h.fetch==="function"?h.fetch:(typeof h==="function"?h:null);if(!fn)return;var res=fn.call(h,new Request("__SLICE_LIVE_REQ__"),globalThis.process.env,__slice_web_ctx);if(res){if(res._isSliceRes){var b=res._body||(res._data!==undefined?JSON.stringify(res._data):"");__slice_http_respond(res.status,b,res._ct,__resHdrJson(res.headers));}else if(typeof res.then==="function")res.then(function(r){if(r&&r._isSliceRes){var rb=r._body||(r._data!==undefined?JSON.stringify(r._data):"");__slice_http_respond(r.status,rb,r._ct,__resHdrJson(r.headers));}});}};globalThis.__slice_builtins=Object.create(null);globalThis.__slice_cjs_require=function(id){return __slice_builtins[id]||__slice_load_builtin(id);};globalThis.require=__slice_cjs_require;"##;
    let _ = ctx.eval(boa_engine::Source::from_bytes(CORE_PRELUDE));
    let _ = ctx.eval(boa_engine::Source::from_bytes(WINTERTC_PHASE_N3_PRELUDE));
    let ses_check_fn = ctx
        .eval(boa_engine::Source::from_bytes(
            b"globalThis.__slice_check_ses",
        ))
        .ok()
        .and_then(|v| v.as_object());
    WarmBoaRealm {
        ctx,
        loaded_bundles: FxHashMap::default(),
        ses_check_fn,
    }
}

const WINTERTC_PHASE_N3_PRELUDE: &[u8] = br##"(function(){
globalThis.btoa = function(s) {
  if (arguments.length === 0) throw new TypeError("btoa requires at least 1 argument");
  return __slice_btoa(String(s));
};
globalThis.atob = function(s) {
  if (arguments.length === 0) throw new TypeError("atob requires at least 1 argument");
  return __slice_atob(String(s));
};

function TextEncoder() {
  this.encoding = "utf-8";
}
TextEncoder.prototype.encode = function(input) {
  return __slice_text_encode(input === undefined ? "" : String(input));
};
TextEncoder.prototype.encodeInto = function(src, dest) {
  return __slice_text_encode_into(String(src), dest);
};
globalThis.TextEncoder = TextEncoder;

function __normEncLabel(label) {
  if (label === undefined) return "utf-8";
  var s = String(label).trim().toLowerCase();
  if (s === "utf-8" || s === "utf8" || s === "unicode-1-1-utf-8") return "utf-8";
  if (s === "ascii" || s === "us-ascii") return "ascii";
  if (s === "latin1" || s === "iso-8859-1" || s === "windows-1252") return "iso-8859-1";
  throw new RangeError("TextDecoder: Unsupported encoding label '" + label + "'");
}
function TextDecoder(label, options) {
  var o = options || {};
  this.encoding = __normEncLabel(label);
  this.fatal = Boolean(o.fatal);
  this.ignoreBOM = Boolean(o.ignoreBOM);
}
TextDecoder.prototype.decode = function(input, options) {
  if (input === undefined || input === null) return "";
  return __slice_text_decode(input, this.encoding, this.fatal, this.ignoreBOM);
};
globalThis.TextDecoder = TextDecoder;

function Event(type, init) {
  if (arguments.length === 0) throw new TypeError("Event constructor requires a type argument");
  var o = init || {};
  this.type = String(type);
  this.bubbles = Boolean(o.bubbles);
  this.cancelable = Boolean(o.cancelable);
  this.composed = Boolean(o.composed);
  this.defaultPrevented = false;
  this.target = null;
  this.currentTarget = null;
  this.srcElement = null;
  this.eventPhase = 0;
  this.isTrusted = false;
  this.timeStamp = performance.now();
  this._stop = false;
  this._stopImm = false;
}
Event.prototype.preventDefault = function() {
  if (this.cancelable) this.defaultPrevented = true;
};
Event.prototype.stopPropagation = function() {
  this._stop = true;
};
Event.prototype.stopImmediatePropagation = function() {
  this._stop = true;
  this._stopImm = true;
};
Event.prototype.composedPath = function() {
  return this.currentTarget ? [this.currentTarget] : [];
};

function CustomEvent(type, init) {
  Event.call(this, type, init);
  this.detail = (init && init.detail !== undefined) ? init.detail : null;
}
CustomEvent.prototype = Object.create(Event.prototype);
CustomEvent.prototype.constructor = CustomEvent;

function EventTarget() {
  this._et = Object.create(null);
}
EventTarget.prototype._ensureEt = function() {
  if (!this._et) this._et = Object.create(null);
  return this._et;
};
EventTarget.prototype.addEventListener = function(type, listener, options) {
  if (listener === null || listener === undefined) return;
  if (typeof listener !== "function" && (typeof listener !== "object" || typeof listener.handleEvent !== "function")) return;
  var o = (typeof options === "object" && options !== null) ? options : { capture: Boolean(options) };
  var sig = o.signal || null;
  if (sig && sig.aborted) return;
  var t = String(type), map = this._ensureEt(), list = map[t] || (map[t] = []);
  for (var i = 0; i < list.length; i++) {
    if (list[i].fn === listener) return;
  }
  list.push({ fn: listener, once: Boolean(o.once) });
  if (sig && typeof sig.addEventListener === "function") {
    var self = this;
    sig.addEventListener("abort", function() {
      self.removeEventListener(t, listener);
    });
  }
};
EventTarget.prototype.removeEventListener = function(type, listener) {
  if (!this._et) return;
  var t = String(type), list = this._et[t];
  if (!list) return;
  var nx = [];
  for (var i = 0; i < list.length; i++) {
    if (list[i].fn !== listener) nx.push(list[i]);
  }
  this._et[t] = nx;
};
EventTarget.prototype.dispatchEvent = function(event) {
  if (!event || typeof event.type !== "string") throw new TypeError("dispatchEvent requires an Event");
  event.target = this;
  event.currentTarget = this;
  event.srcElement = this;
  var t = event.type;
  var onProp = this["on" + t];
  if (typeof onProp === "function") {
    try { onProp.call(this, event); } catch (_e) {}
  }
  if (this._et && this._et[t]) {
    var snapshot = this._et[t].slice();
    for (var i = 0; i < snapshot.length; i++) {
      if (event._stopImm) break;
      var item = snapshot[i];
      if (item.once) this.removeEventListener(t, item.fn);
      try {
        if (typeof item.fn === "function") item.fn.call(this, event);
        else if (item.fn && typeof item.fn.handleEvent === "function") item.fn.handleEvent(event);
      } catch (_e2) {}
    }
  }
  return !event.defaultPrevented;
};
globalThis.Event = Event;
globalThis.CustomEvent = CustomEvent;
globalThis.EventTarget = EventTarget;

AbortSignal.prototype.dispatchEvent = function(ev) {
  if (ev && ev.type === "abort") {
    this._fire(this.reason);
    return true;
  }
  return true;
};
AbortSignal.any = function(signals) {
  if (!signals || typeof signals.length !== "number") {
    throw new TypeError("AbortSignal.any requires an array or iterable of AbortSignal instances");
  }
  var s = new AbortSignal();
  for (var i = 0; i < signals.length; i++) {
    var sig = signals[i];
    if (sig && sig.aborted) {
      s._fire(sig.reason);
      return s;
    }
  }
  for (var j = 0; j < signals.length; j++) {
    (function(sig) {
      if (sig && typeof sig.addEventListener === "function") {
        sig.addEventListener("abort", function() {
          s._fire(sig.reason);
        });
      }
    })(signals[j]);
  }
  return s;
};

function __algName(alg) {
  if (typeof alg === "string") return alg.trim().toUpperCase();
  if (alg && typeof alg === "object" && typeof alg.name === "string") return alg.name.trim().toUpperCase();
  return "";
}
function __hmacHashName(alg) {
  if (!alg || typeof alg !== "object") return "SHA-256";
  var h = alg.hash;
  if (typeof h === "string") return h.trim().toUpperCase();
  if (h && typeof h === "object" && typeof h.name === "string") return h.name.trim().toUpperCase();
  return "SHA-256";
}
function CryptoKey(type, extractable, algorithm, usages, rawU8) {
  this.type = type;
  this.extractable = Boolean(extractable);
  this.algorithm = algorithm;
  this.usages = Array.isArray(usages) ? usages.slice() : [];
  this._raw = rawU8;
}
globalThis.CryptoKey = CryptoKey;

function __toU8(input) {
  if (input === undefined || input === null) return new Uint8Array(0);
  if (input instanceof Uint8Array) return input;
  if (input instanceof ArrayBuffer) return new Uint8Array(input);
  if (input.buffer instanceof ArrayBuffer) {
    return new Uint8Array(input.buffer, input.byteOffset || 0, input.byteLength);
  }
  if (input instanceof Blob) return input._u8;
  if (typeof input === "string") return __slice_text_encode(input);
  if (Array.isArray(input)) return new Uint8Array(input);
  return __slice_text_encode(String(input));
}
function __concatU8(chunks) {
  if (chunks.length === 0) return new Uint8Array(0);
  if (chunks.length === 1) return new Uint8Array(chunks[0]);
  var total = 0;
  for (var i = 0; i < chunks.length; i++) total += chunks[i].byteLength;
  var out = new Uint8Array(total), off = 0;
  for (var j = 0; j < chunks.length; j++) {
    out.set(chunks[j], off);
    off += chunks[j].byteLength;
  }
  return out;
}

var __subtle = {
  digest: function(algorithm, data) {
    try {
      var name = __algName(algorithm);
      var ab = __slice_crypto_digest(name, data);
      return Promise.resolve(ab);
    } catch (e) {
      return Promise.reject(e);
    }
  },
  importKey: function(format, keyData, algorithm, extractable, keyUsages) {
    try {
      var fmt = String(format).toLowerCase();
      var name = __algName(algorithm);
      if (name !== "HMAC") {
        throw new Error("NotSupportedError: Unsupported importKey algorithm '" + name + "'");
      }
      var hashName = __hmacHashName(algorithm);
      if (hashName === "SHA256") hashName = "SHA-256";
      else if (hashName === "SHA384") hashName = "SHA-384";
      else if (hashName === "SHA512") hashName = "SHA-512";
      else if (hashName === "SHA1") hashName = "SHA-1";
      var rawU8;
      if (fmt === "raw") {
        rawU8 = new Uint8Array(__toU8(keyData));
      } else if (fmt === "jwk") {
        if (!keyData || typeof keyData.k !== "string") {
          throw new TypeError("DataError: Invalid JWK keyData for HMAC");
        }
        rawU8 = __slice_base64_decode_bytes(keyData.k);
      } else {
        throw new Error("NotSupportedError: Unsupported key format '" + fmt + "'");
      }
      if (rawU8.byteLength === 0) {
        throw new TypeError("DataError: HMAC key length cannot be zero");
      }
      var keyAlg = { name: "HMAC", hash: { name: hashName }, length: rawU8.byteLength * 8 };
      return Promise.resolve(new CryptoKey("secret", extractable, keyAlg, keyUsages, rawU8));
    } catch (e) {
      return Promise.reject(e);
    }
  },
  exportKey: function(format, key) {
    try {
      if (!key || !(key instanceof CryptoKey)) throw new TypeError("Invalid CryptoKey");
      if (!key.extractable) throw new Error("InvalidAccessError: key is not extractable");
      var fmt = String(format).toLowerCase();
      if (fmt === "raw") {
        var copy = new Uint8Array(key._raw);
        return Promise.resolve(copy.buffer);
      }
      if (fmt === "jwk") {
        var hName = (key.algorithm && key.algorithm.hash && key.algorithm.hash.name) || "SHA-256";
        var jwkAlg = hName === "SHA-512" ? "HS512" : (hName === "SHA-384" ? "HS384" : (hName === "SHA-1" ? "HS1" : "HS256"));
        return Promise.resolve({
          kty: "oct",
          k: __slice_base64_encode_bytes(key._raw, true),
          alg: jwkAlg,
          ext: true,
          key_ops: key.usages.slice()
        });
      }
      throw new Error("NotSupportedError: Unsupported exportKey format '" + fmt + "'");
    } catch (e) {
      return Promise.reject(e);
    }
  },
  sign: function(algorithm, key, data) {
    try {
      if (!key || !(key instanceof CryptoKey)) throw new TypeError("Invalid CryptoKey");
      if (key.usages.indexOf("sign") === -1) {
        throw new Error("InvalidAccessError: key does not support 'sign' usage");
      }
      var hashName = (key.algorithm && key.algorithm.hash && key.algorithm.hash.name) || __hmacHashName(algorithm);
      var ab = __slice_crypto_hmac_sign(hashName, key._raw, data);
      return Promise.resolve(ab);
    } catch (e) {
      return Promise.reject(e);
    }
  },
  verify: function(algorithm, key, signature, data) {
    try {
      if (!key || !(key instanceof CryptoKey)) throw new TypeError("Invalid CryptoKey");
      if (key.usages.indexOf("verify") === -1) {
        throw new Error("InvalidAccessError: key does not support 'verify' usage");
      }
      var hashName = (key.algorithm && key.algorithm.hash && key.algorithm.hash.name) || __hmacHashName(algorithm);
      var ok = __slice_crypto_hmac_verify(hashName, key._raw, signature, data);
      return Promise.resolve(Boolean(ok));
    } catch (e) {
      return Promise.reject(e);
    }
  },
  timingSafeEqual: function(a, b) {
    return __slice_crypto_timing_safe_equal(a, b);
  }
};

globalThis.crypto = {
  randomUUID: function() {
    return __slice_crypto_random_uuid();
  },
  getRandomValues: function(typedArray) {
    return __slice_crypto_get_random_values(typedArray);
  },
  subtle: __subtle
};

function Blob(blobParts, options) {
  var o = options || {};
  this.type = o.type ? String(o.type).toLowerCase() : "";
  var chunks = [];
  if (blobParts !== undefined && blobParts !== null) {
    if (!Array.isArray(blobParts) && typeof blobParts.length !== "number") {
      throw new TypeError("Blob parts must be an sequence/array");
    }
    for (var i = 0; i < blobParts.length; i++) {
      chunks.push(__toU8(blobParts[i]));
    }
  }
  this._u8 = __concatU8(chunks);
  this.size = this._u8.byteLength;
}
Blob.prototype.text = function() {
  return Promise.resolve(__slice_text_decode(this._u8, "utf-8", false, false));
};
Blob.prototype.arrayBuffer = function() {
  var copy = new Uint8Array(this._u8);
  return Promise.resolve(copy.buffer);
};
Blob.prototype.bytes = function() {
  return Promise.resolve(new Uint8Array(this._u8));
};
Blob.prototype.slice = function(start, end, contentType) {
  var len = this.size;
  var s = start === undefined ? 0 : (start < 0 ? Math.max(len + Number(start), 0) : Math.min(Number(start), len));
  var e = end === undefined ? len : (end < 0 ? Math.max(len + Number(end), 0) : Math.min(Number(end), len));
  var sub = e > s ? this._u8.subarray(s, e) : new Uint8Array(0);
  return new Blob([sub], { type: contentType !== undefined ? String(contentType) : "" });
};
Blob.prototype.stream = function() {
  var u8 = new Uint8Array(this._u8);
  return new ReadableStream({
    start: function(c) {
      if (u8.byteLength > 0) c.enqueue(u8);
      c.close();
    }
  });
};
globalThis.Blob = Blob;

function File(fileBits, fileName, options) {
  if (arguments.length < 2) throw new TypeError("File constructor requires at least 2 arguments");
  Blob.call(this, fileBits, options);
  this.name = String(fileName);
  this.lastModified = (options && options.lastModified !== undefined) ? Number(options.lastModified) : Date.now();
}
File.prototype = Object.create(Blob.prototype);
File.prototype.constructor = File;
globalThis.File = File;

function FormData() {
  this._p = [];
}
FormData.prototype._normVal = function(value, filename) {
  if (value instanceof File) {
    return filename !== undefined ? new File([value._u8], filename, { type: value.type, lastModified: value.lastModified }) : value;
  }
  if (value instanceof Blob) {
    return new File([value._u8], filename !== undefined ? String(filename) : "blob", { type: value.type });
  }
  return String(value);
};
FormData.prototype.append = function(name, value, filename) {
  this._p.push([String(name), this._normVal(value, filename)]);
};
FormData.prototype.set = function(name, value, filename) {
  var k = String(name), v = this._normVal(value, filename), fi = -1;
  for (var i = 0; i < this._p.length; i++) {
    if (this._p[i][0] === k) {
      if (fi === -1) { this._p[i][1] = v; fi = i; }
      else { this._p.splice(i, 1); i--; }
    }
  }
  if (fi === -1) this._p.push([k, v]);
};
FormData.prototype.get = function(name) {
  var k = String(name);
  for (var i = 0; i < this._p.length; i++) if (this._p[i][0] === k) return this._p[i][1];
  return null;
};
FormData.prototype.getAll = function(name) {
  var k = String(name), r = [];
  for (var i = 0; i < this._p.length; i++) if (this._p[i][0] === k) r.push(this._p[i][1]);
  return r;
};
FormData.prototype.has = function(name) {
  return this.get(name) !== null;
};
FormData.prototype.delete = function(name) {
  var k = String(name), nx = [];
  for (var i = 0; i < this._p.length; i++) if (this._p[i][0] !== k) nx.push(this._p[i]);
  this._p = nx;
};
FormData.prototype.entries = function() {
  return this._p.slice();
};
FormData.prototype.keys = function() {
  var r = [];
  for (var i = 0; i < this._p.length; i++) r.push(this._p[i][0]);
  return r;
};
FormData.prototype.values = function() {
  var r = [];
  for (var i = 0; i < this._p.length; i++) r.push(this._p[i][1]);
  return r;
};
FormData.prototype.forEach = function(cb, thisArg) {
  for (var i = 0; i < this._p.length; i++) cb.call(thisArg, this._p[i][1], this._p[i][0], this);
};
if (typeof Symbol !== "undefined" && Symbol.iterator) {
  FormData.prototype[Symbol.iterator] = function() {
    return this._p[Symbol.iterator]();
  };
}
globalThis.FormData = FormData;

function __extractBodyBytes(obj) {
  if (obj._u8 instanceof Uint8Array) return new Uint8Array(obj._u8);
  var s = obj._body || (obj._data !== undefined ? JSON.stringify(obj._data) : "");
  return __slice_text_encode(s);
}
function __bodyToFormData(obj) {
  if (obj._fd instanceof FormData) return obj._fd;
  var fd = new FormData();
  var usp = new URLSearchParams(obj._body || "");
  var es = usp.entries();
  for (var i = 0; i < es.length; i++) fd.append(es[i][0], es[i][1]);
  return fd;
}
var __origReq = globalThis.Request;
function __normHttpBody(rawBody, headers) {
  if (rawBody === undefined || rawBody === null) return { str: "", u8: null, fd: null };
  if (typeof rawBody === "string") return { str: rawBody, u8: null, fd: null };
  if (rawBody instanceof URLSearchParams) {
    if (headers && !headers.has("content-type")) headers.set("content-type", "application/x-www-form-urlencoded;charset=UTF-8");
    return { str: rawBody.toString(), u8: null, fd: null };
  }
  if (rawBody instanceof FormData) {
    if (headers && !headers.has("content-type")) headers.set("content-type", "multipart/form-data; boundary=----SliceFormBoundary");
    var pairs = [], es = rawBody.entries();
    for (var i = 0; i < es.length; i++) {
      var v = es[i][1];
      pairs.push(encodeURIComponent(es[i][0]) + "=" + encodeURIComponent(v instanceof Blob ? __slice_text_decode(v._u8, "utf-8", false, false) : String(v)));
    }
    return { str: pairs.join("&"), u8: null, fd: rawBody };
  }
  if (rawBody instanceof Blob) {
    if (rawBody.type && headers && !headers.has("content-type")) headers.set("content-type", rawBody.type);
    var u8b = new Uint8Array(rawBody._u8);
    return { str: __slice_text_decode(u8b, "utf-8", false, false), u8: u8b, fd: null };
  }
  if (rawBody instanceof Uint8Array || rawBody instanceof ArrayBuffer || (rawBody && rawBody.buffer instanceof ArrayBuffer)) {
    var u8 = new Uint8Array(__toU8(rawBody));
    return { str: __slice_text_decode(u8, "utf-8", false, false), u8: u8, fd: null };
  }
  if (rawBody instanceof ReadableStream) {
    var txt = rawBody._toText();
    return { str: txt, u8: rawBody._toBytes ? rawBody._toBytes() : __slice_text_encode(txt), fd: null };
  }
  return { str: JSON.stringify(rawBody), u8: null, fd: null };
}

function Request(input, init) {
  if (input === "__SLICE_LIVE_REQ__") {
    __origReq.call(this, input, init);
    this._u8 = null;
    this._fd = null;
    return;
  }
  var o = init || {}, isReq = input instanceof Request;
  __origReq.call(this, input, o);
  if (o.body !== undefined && o.body !== null) {
    var nb = __normHttpBody(o.body, this.headers);
    this._body = nb.str;
    this._u8 = nb.u8;
    this._fd = nb.fd;
  } else if (isReq) {
    this._u8 = input._u8 ? new Uint8Array(input._u8) : null;
    this._fd = input._fd || null;
  } else {
    this._u8 = null;
    this._fd = null;
  }
}
Request.prototype = __origReq.prototype;
Request.prototype.constructor = Request;
Request.prototype.arrayBuffer = function() {
  this.bodyUsed = true;
  return Promise.resolve(__extractBodyBytes(this).buffer);
};
Request.prototype.bytes = function() {
  this.bodyUsed = true;
  return Promise.resolve(__extractBodyBytes(this));
};
Request.prototype.blob = function() {
  this.bodyUsed = true;
  return Promise.resolve(new Blob([__extractBodyBytes(this)], { type: this.headers.get("content-type") || "" }));
};
Request.prototype.formData = function() {
  this.bodyUsed = true;
  return Promise.resolve(__bodyToFormData(this));
};
Object.defineProperty(Request.prototype, "body", {
  get: function() {
    var b = __extractBodyBytes(this);
    return new ReadableStream({
      start: function(c) {
        if (b.byteLength > 0) c.enqueue(b);
        c.close();
      }
    });
  }
});
globalThis.Request = Request;

var __origRes = globalThis.Response;
function Response(body, init) {
  var o = init || {};
  __origRes.call(this, null, o);
  var nb = __normHttpBody(body, this.headers);
  this._body = nb.str;
  this._u8 = nb.u8;
  this._fd = nb.fd;
  this._ct = (this.headers._m && this.headers._m["content-type"]) || "text/plain;charset=UTF-8";
}
Response.prototype = __origRes.prototype;
Response.prototype.constructor = Response;
Response.json = __origRes.json;
Response.sse = __origRes.sse;
Response.redirect = __origRes.redirect;
Response.error = function() {
  var r = new Response("", { status: 0, statusText: "" });
  r.ok = false;
  r.type = "error";
  return r;
};
Response.prototype.arrayBuffer = function() {
  this.bodyUsed = true;
  return Promise.resolve(__extractBodyBytes(this).buffer);
};
Response.prototype.bytes = function() {
  this.bodyUsed = true;
  return Promise.resolve(__extractBodyBytes(this));
};
Response.prototype.blob = function() {
  this.bodyUsed = true;
  return Promise.resolve(new Blob([__extractBodyBytes(this)], { type: this._ct || "" }));
};
Response.prototype.formData = function() {
  this.bodyUsed = true;
  return Promise.resolve(__bodyToFormData(this));
};
Object.defineProperty(Response.prototype, "body", {
  get: function() {
    var b = __extractBodyBytes(this);
    return new ReadableStream({
      start: function(c) {
        if (b.byteLength > 0) c.enqueue(b);
        c.close();
      }
    });
  }
});
globalThis.Response = Response;

var MAX_STREAM_CHUNKS = 4096;
function ReadableStream(underlyingSource) {
  var self = this;
  this._chunks = [];
  this._closed = false;
  this._errored = null;
  this.locked = false;
  this._src = underlyingSource || null;
  var ctrl = {
    desiredSize: 1,
    enqueue: function(chunk) {
      if (self._closed) throw new TypeError("ReadableStream is already closed");
      if (self._chunks.length >= MAX_STREAM_CHUNKS) {
        throw new RangeError("ReadableStream buffer limit exceeded (" + MAX_STREAM_CHUNKS + " chunks)");
      }
      self._chunks.push(chunk);
    },
    close: function() {
      self._closed = true;
    },
    error: function(e) {
      self._errored = e || new Error("ReadableStream error");
      self._closed = true;
    }
  };
  this._ctrl = ctrl;
  if (this._src && typeof this._src.start === "function") {
    this._src.start(ctrl);
  }
}
ReadableStream.from = function(iterable) {
  return new ReadableStream({
    start: function(c) {
      if (Array.isArray(iterable)) {
        for (var i = 0; i < iterable.length; i++) c.enqueue(iterable[i]);
      } else if (iterable && typeof iterable[Symbol.iterator] === "function") {
        var it = iterable[Symbol.iterator](), step = it.next();
        while (!step.done) { c.enqueue(step.value); step = it.next(); }
      }
      c.close();
    }
  });
};
ReadableStream.prototype._drainSync = function() {
  var guard = 0;
  while (!this._closed && this._src && typeof this._src.pull === "function" && guard < MAX_STREAM_CHUNKS) {
    guard++;
    var before = this._chunks.length;
    this._src.pull(this._ctrl);
    if (this._chunks.length === before && !this._closed) break;
  }
};
ReadableStream.prototype._toText = function() {
  this._drainSync();
  var out = "";
  for (var i = 0; i < this._chunks.length; i++) {
    var c = this._chunks[i];
    if (typeof c === "string") out += c;
    else if (c instanceof Uint8Array || c instanceof ArrayBuffer || (c && c.buffer instanceof ArrayBuffer)) {
      out += __slice_text_decode(c, "utf-8", false, false);
    } else if (c !== undefined && c !== null) {
      out += String(c);
    }
  }
  return out;
};
ReadableStream.prototype._toBytes = function() {
  this._drainSync();
  var u8s = [];
  for (var i = 0; i < this._chunks.length; i++) u8s.push(__toU8(this._chunks[i]));
  return __concatU8(u8s);
};
ReadableStream.prototype.cancel = function(reason) {
  this._closed = true;
  this._chunks = [];
  if (this._src && typeof this._src.cancel === "function") {
    try { return Promise.resolve(this._src.cancel(reason)); } catch (e) { return Promise.reject(e); }
  }
  return Promise.resolve(undefined);
};
ReadableStream.prototype.getReader = function() {
  if (this.locked) throw new TypeError("ReadableStream is already locked to a reader");
  this.locked = true;
  var self = this, idx = 0;
  return {
    closed: Promise.resolve(undefined),
    read: function() {
      if (self._errored) return Promise.reject(self._errored);
      if (idx < self._chunks.length) {
        return Promise.resolve({ value: self._chunks[idx++], done: false });
      }
      if (!self._closed && self._src && typeof self._src.pull === "function") {
        try {
          var p = self._src.pull(self._ctrl);
          if (p && typeof p.then === "function") {
            return p.then(function() {
              if (self._errored) throw self._errored;
              if (idx < self._chunks.length) return { value: self._chunks[idx++], done: false };
              return { value: undefined, done: true };
            });
          }
        } catch (e) {
          return Promise.reject(e);
        }
        if (idx < self._chunks.length) {
          return Promise.resolve({ value: self._chunks[idx++], done: false });
        }
      }
      return Promise.resolve({ value: undefined, done: true });
    },
    cancel: function(reason) {
      return self.cancel(reason);
    },
    releaseLock: function() {
      self.locked = false;
    }
  };
};
ReadableStream.prototype.tee = function() {
  this._drainSync();
  var copy1 = this._chunks.slice(), copy2 = this._chunks.slice();
  return [
    ReadableStream.from(copy1),
    ReadableStream.from(copy2)
  ];
};
ReadableStream.prototype.pipeTo = function(writable) {
  var reader = this.getReader();
  var writer = writable.getWriter();
  function pump() {
    return reader.read().then(function(step) {
      if (step.done) {
        reader.releaseLock();
        return writer.close().then(function() { writer.releaseLock(); });
      }
      return writer.write(step.value).then(pump);
    });
  }
  return pump();
};
ReadableStream.prototype.pipeThrough = function(transform) {
  if (!transform || !transform.readable || !transform.writable) {
    throw new TypeError("pipeThrough requires a { readable, writable } TransformStream pair");
  }
  this._drainSync();
  var writer = transform.writable.getWriter();
  for (var i = 0; i < this._chunks.length; i++) {
    writer.write(this._chunks[i]);
  }
  writer.close();
  writer.releaseLock();
  return transform.readable;
};
globalThis.ReadableStream = ReadableStream;

function WritableStream(underlyingSink) {
  var self = this;
  this._sink = underlyingSink || null;
  this._closed = false;
  this.locked = false;
  this._ctrl = {
    error: function(e) { self._errored = e || new Error("WritableStream error"); }
  };
  if (this._sink && typeof this._sink.start === "function") {
    this._sink.start(this._ctrl);
  }
}
WritableStream.prototype.abort = function(reason) {
  this._closed = true;
  if (this._sink && typeof this._sink.abort === "function") {
    try { return Promise.resolve(this._sink.abort(reason)); } catch (e) { return Promise.reject(e); }
  }
  return Promise.resolve(undefined);
};
WritableStream.prototype.close = function() {
  if (this._closed) return Promise.reject(new TypeError("WritableStream is already closed"));
  this._closed = true;
  if (this._sink && typeof this._sink.close === "function") {
    try { return Promise.resolve(this._sink.close()); } catch (e) { return Promise.reject(e); }
  }
  return Promise.resolve(undefined);
};
WritableStream.prototype.getWriter = function() {
  if (this.locked) throw new TypeError("WritableStream is already locked to a writer");
  this.locked = true;
  var self = this;
  return {
    ready: Promise.resolve(undefined),
    closed: Promise.resolve(undefined),
    write: function(chunk) {
      if (self._closed) return Promise.reject(new TypeError("WritableStream is closed"));
      if (self._sink && typeof self._sink.write === "function") {
        try {
          return Promise.resolve(self._sink.write(chunk, self._ctrl));
        } catch (e) {
          return Promise.reject(e);
        }
      }
      return Promise.resolve(undefined);
    },
    close: function() {
      return self.close();
    },
    abort: function(reason) {
      return self.abort(reason);
    },
    releaseLock: function() {
      self.locked = false;
    }
  };
};
globalThis.WritableStream = WritableStream;

function TransformStream(transformer) {
  var tr = transformer || {};
  var readable = new ReadableStream();
  var rCtrl = readable._ctrl;
  var tCtrl = {
    desiredSize: 1,
    enqueue: function(chunk) { rCtrl.enqueue(chunk); },
    error: function(e) { rCtrl.error(e); },
    terminate: function() { rCtrl.close(); }
  };
  if (typeof tr.start === "function") {
    tr.start(tCtrl);
  }
  var writable = new WritableStream({
    write: function(chunk) {
      if (typeof tr.transform === "function") {
        return tr.transform(chunk, tCtrl);
      }
      tCtrl.enqueue(chunk);
    },
    close: function() {
      var res = typeof tr.flush === "function" ? tr.flush(tCtrl) : undefined;
      if (res && typeof res.then === "function") {
        return res.then(function() { rCtrl.close(); });
      }
      rCtrl.close();
    },
    abort: function(reason) {
      rCtrl.error(reason);
    }
  });
  this.readable = readable;
  this.writable = writable;
}
globalThis.TransformStream = TransformStream;

function TextEncoderStream() {
  var enc = new TextEncoder();
  var ts = new TransformStream({
    transform: function(chunk, ctrl) {
      var s = String(chunk);
      if (s.length > 0) ctrl.enqueue(enc.encode(s));
    }
  });
  this.encoding = "utf-8";
  this.readable = ts.readable;
  this.writable = ts.writable;
}
globalThis.TextEncoderStream = TextEncoderStream;

function TextDecoderStream(label, options) {
  var dec = new TextDecoder(label, options);
  var ts = new TransformStream({
    transform: function(chunk, ctrl) {
      var s = dec.decode(chunk);
      if (s.length > 0) ctrl.enqueue(s);
    }
  });
  this.encoding = dec.encoding;
  this.fatal = dec.fatal;
  this.ignoreBOM = dec.ignoreBOM;
  this.readable = ts.readable;
  this.writable = ts.writable;
}
globalThis.TextDecoderStream = TextDecoderStream;

function __cloneDeep(val, seenSrc, seenDst) {
  if (val === null || typeof val !== "object") {
    if (typeof val === "function" || typeof val === "symbol") {
      throw new Error("DataCloneError: Function or Symbol cannot be cloned by structuredClone");
    }
    return val;
  }
  for (var i = 0; i < seenSrc.length; i++) {
    if (seenSrc[i] === val) return seenDst[i];
  }
  if (val instanceof Date) return new Date(val.getTime());
  if (val instanceof RegExp) return new RegExp(val.source, val.flags);
  if (val instanceof ArrayBuffer) {
    var abCopy = val.slice(0);
    seenSrc.push(val); seenDst.push(abCopy);
    return abCopy;
  }
  if (val.buffer instanceof ArrayBuffer && typeof val.byteLength === "number") {
    var Ctor = val.constructor;
    var sliceBuf = val.buffer.slice(val.byteOffset || 0, (val.byteOffset || 0) + val.byteLength);
    var viewCopy = new Ctor(sliceBuf);
    seenSrc.push(val); seenDst.push(viewCopy);
    return viewCopy;
  }
  if (val instanceof File) {
    var fCopy = new File([new Uint8Array(val._u8)], val.name, { type: val.type, lastModified: val.lastModified });
    seenSrc.push(val); seenDst.push(fCopy);
    return fCopy;
  }
  if (val instanceof Blob) {
    var bCopy = new Blob([new Uint8Array(val._u8)], { type: val.type });
    seenSrc.push(val); seenDst.push(bCopy);
    return bCopy;
  }
  if (val instanceof Map) {
    var mCopy = new Map();
    seenSrc.push(val); seenDst.push(mCopy);
    val.forEach(function(v, k) {
      mCopy.set(__cloneDeep(k, seenSrc, seenDst), __cloneDeep(v, seenSrc, seenDst));
    });
    return mCopy;
  }
  if (val instanceof Set) {
    var sCopy = new Set();
    seenSrc.push(val); seenDst.push(sCopy);
    val.forEach(function(v) {
      sCopy.add(__cloneDeep(v, seenSrc, seenDst));
    });
    return sCopy;
  }
  if (val instanceof Error) {
    var eCopy = new Error(val.message);
    eCopy.name = val.name;
    if (val.stack) eCopy.stack = val.stack;
    seenSrc.push(val); seenDst.push(eCopy);
    return eCopy;
  }
  if (Array.isArray(val)) {
    var arrCopy = new Array(val.length);
    seenSrc.push(val); seenDst.push(arrCopy);
    for (var j = 0; j < val.length; j++) {
      arrCopy[j] = __cloneDeep(val[j], seenSrc, seenDst);
    }
    return arrCopy;
  }
  var objCopy = {};
  seenSrc.push(val); seenDst.push(objCopy);
  var keys = Object.keys(val);
  for (var k = 0; k < keys.length; k++) {
    var prop = keys[k];
    objCopy[prop] = __cloneDeep(val[prop], seenSrc, seenDst);
  }
  return objCopy;
}
globalThis.structuredClone = function(value) {
  if (arguments.length === 0) throw new TypeError("structuredClone requires at least 1 argument");
  return __cloneDeep(value, [], []);
};
})();"##;

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
                "crypto" | "node:crypto" => br#"var __h={_c:"",_b:false,update:function(d){this._c+=String(d);return this;},digest:function(){var r=__slice_crypto_sha256(this._c);this._c="";this._b=false;return r;}},__m={_k:"",_c:"",_b:false,update:function(d){this._c+=String(d);return this;},digest:function(){var r=__slice_crypto_hmac_sha256(this._k,this._c);this._c="";this._b=false;return r;}},m={createHash:function(){if(!__h._b){__h._c="";__h._b=true;return __h;}return{_c:"",update:function(d){this._c+=String(d);return this;},digest:function(){return __slice_crypto_sha256(this._c);}};},createHmac:function(_a,s){var k=String(s);if(!__m._b){__m._k=k;__m._c="";__m._b=true;return __m;}return{_c:"",update:function(d){this._c+=String(d);return this;},digest:function(){return __slice_crypto_hmac_sha256(k,this._c);}};},randomUUID:function(){return __slice_crypto_random_uuid();},getRandomValues:function(ta){return __slice_crypto_get_random_values(ta);},timingSafeEqual:function(a,b){return __slice_crypto_timing_safe_equal(a,b);},subtle:globalThis.crypto.subtle,webcrypto:globalThis.crypto};m.default=m;m.__esModule=true;__slice_builtins["crypto"]=__slice_builtins["node:crypto"]=m;"#,
                "fs" | "node:fs" => br#"var m={readFileSync:function(p){return __slice_fs_read_file_sync(String(p));},writeFileSync:function(p,d){return __slice_fs_write_file_sync(String(p),String(d));},appendFileSync:function(p,d){return __slice_fs_append_file_sync(String(p),String(d));},existsSync:function(p){return Boolean(__slice_fs_exists_sync(String(p)));},mkdirSync:function(p){return __slice_fs_mkdir_sync(String(p));},readdirSync:function(p){return JSON.parse(__slice_fs_readdir_sync(String(p)));},statSync:function(p){return{size:__slice_fs_stat_sync(String(p))};},unlinkSync:function(p){return __slice_fs_rm_sync(String(p));},rmSync:function(p){return __slice_fs_rm_sync(String(p));}};m.default=m;m.__esModule=true;__slice_builtins["fs"]=__slice_builtins["node:fs"]=m;"#,
                "path" | "node:path" => br#"var m={sep:"/",join:function(){return __slice_path_join.apply(null,arguments);},resolve:function(){return __slice_path_join.apply(null,arguments);},basename:function(p,ext){var s=String(p).replace(/\\/g,"/").replace(/\/+$/,""),i=s.lastIndexOf("/"),b=i>=0?s.slice(i+1):s;return(ext&&b.endsWith(ext))?b.slice(0,b.length-ext.length):b;},dirname:function(p){var s=String(p).replace(/\\/g,"/").replace(/\/+$/,""),i=s.lastIndexOf("/");return i>0?s.slice(0,i):(i===0?"/":".");},extname:function(p){var b=m.basename(p),i=b.lastIndexOf(".");return i>0?b.slice(i):"";}};m.default=m;m.__esModule=true;__slice_builtins["path"]=__slice_builtins["node:path"]=m;"#,
                "os" | "node:os" => br#"var m={tmpdir:function(){return __slice_os_tmpdir();},platform:function(){return __slice_os_platform();},arch:function(){return __slice_os_arch();},EOL:"\n"};m.default=m;m.__esModule=true;__slice_builtins["os"]=__slice_builtins["node:os"]=m;"#,
                "util" | "node:util" => br#"var m={inspect:function(v){return typeof v==="object"?JSON.stringify(v):String(v);},deprecate:function(f){return f;},promisify:function(f){return f;}};m.default=m;__slice_builtins["util"]=__slice_builtins["node:util"]=m;"#,
                "buffer" | "node:buffer" => br#"var m={Buffer:globalThis.Buffer,default:{Buffer:globalThis.Buffer}};__slice_builtins["buffer"]=__slice_builtins["node:buffer"]=m;"#,
                "url" | "node:url" => br#"var m={URL:globalThis.URL,URLSearchParams:globalThis.URLSearchParams,default:{URL:globalThis.URL,URLSearchParams:globalThis.URLSearchParams}};m.__esModule=true;__slice_builtins["url"]=__slice_builtins["node:url"]=m;"#,
                "process" | "node:process" => br#"__slice_builtins["process"]=__slice_builtins["node:process"]=globalThis.process;"#,
                "slice:semver" => br#"var m={valid:function(v){return __slice_semver_valid(v);},coerce:function(v){var c=__slice_semver_coerce(v);return c?{version:c,toString:function(){return c;}}:null;},gt:function(a,b){return Boolean(__slice_semver_gt(a,b));},satisfies:function(v,r){return __slice_semver_satisfies(v,r)===1;}};m.default=m;m.__esModule=true;__slice_builtins["slice:semver"]=m;"#,
                "slice:ms" => br#"var m=function(v,o){return __slice_fast_ms(v,Boolean(o&&o.long));};m.default=m;__slice_builtins["slice:ms"]=m;"#,
                "slice:test" | "node:test" | "bun:test" | "test" => br#"var __st={suites:[],be:[[]],ae:[[]],res:[],passed:0,failed:0,skipped:0,rollbacks:0};function __eq(a,b){if(a===b)return true;if(a&&b&&typeof a==="object"&&typeof b==="object")return JSON.stringify(a)===JSON.stringify(b);return false;}function __vMatch(v,s){if(!s)return true;var q=String(s);if(v.indexOf(q)!==-1)return true;if(q==="FsReadDenied")return v.indexOf("filesystem read authority")!==-1;if(q==="FsWriteDenied")return v.indexOf("filesystem write authority")!==-1;if(q==="NetAccessDenied"||q==="NetConnectDenied")return v.indexOf("outbound network authority")!==-1;if(q==="EnvAccessDenied")return v.indexOf("environment variable authority")!==-1;if(q==="CpuFuelExhausted")return v.indexOf("CPU instruction fuel limit")!==-1;if(q==="MemoryCeilingExceeded"||q==="MemoryLimitExceeded")return v.indexOf("memory ceiling")!==-1;return false;}function __mkExp(act,neg){function chk(ok,msg){if(neg?ok:!ok)throw new Error(msg);}return{get not(){return __mkExp(act,!neg);},toBe:function(e){chk(act===e,"Expected "+JSON.stringify(e)+" but got "+JSON.stringify(act));},toEqual:function(e){chk(__eq(act,e),"Expected deep equal "+JSON.stringify(e)+" but got "+JSON.stringify(act));},toContain:function(s){var ok=(typeof act==="string"||Array.isArray(act))&&act.indexOf(s)!==-1;chk(ok,"Expected "+JSON.stringify(act)+" to contain "+JSON.stringify(s));},toBeTruthy:function(){chk(Boolean(act),"Expected truthy but got "+JSON.stringify(act));},toBeFalsy:function(){chk(!act,"Expected falsy but got "+JSON.stringify(act));},toBeNull:function(){chk(act===null,"Expected null but got "+JSON.stringify(act));},toBeDefined:function(){chk(act!==undefined,"Expected defined value");},toBeUndefined:function(){chk(act===undefined,"Expected undefined but got "+JSON.stringify(act));},toBeGreaterThan:function(n){chk(Number(act)>Number(n),"Expected "+act+" > "+n);},toBeGreaterThanOrEqual:function(n){chk(Number(act)>=Number(n),"Expected "+act+" >= "+n);},toBeLessThan:function(n){chk(Number(act)<Number(n),"Expected "+act+" < "+n);},toBeLessThanOrEqual:function(n){chk(Number(act)<=Number(n),"Expected "+act+" <= "+n);},toHaveLength:function(n){chk(act!=null&&act.length===n,"Expected length "+n+" but got "+(act&&act.length));},toHaveProperty:function(k,v){var hp=act!=null&&typeof act==="object"&&(k in act);if(hp&&v!==undefined)hp=__eq(act[k],v);chk(hp,"Expected property "+k);},toThrow:function(sub){var th=false,em="";try{act();}catch(e){th=true;em=String(e&&e.message||e);}if(th&&sub)th=em.indexOf(String(sub))!==-1;chk(th,"Expected function to throw"+(sub?" matching "+sub:""));},toViolate:function(sub){var r=typeof act==="function"?act():act,v=r&&r.violation?String(r.violation):"";var ok=Boolean(r&&r.ok===false&&v.length>0&&__vMatch(v,sub));chk(ok,"Expected sandbox violation "+(sub||"*")+" but got "+(v||"ok:true"));},toStayWithinFuel:function(maxF){var r=typeof act==="function"?act():act;var ok=Boolean(r&&r.ok===true&&typeof r.fuelUsed==="number"&&r.fuelUsed<=Number(maxF));chk(ok,"Expected fuelUsed <= "+maxF+" but got "+(r&&r.fuelUsed));},toStayWithinMemory:function(maxB){var r=typeof act==="function"?act():act;var ok=Boolean(r&&r.ok===true&&typeof r.memoryUsedBytes==="number"&&r.memoryUsedBytes<=Number(maxB));chk(ok,"Expected memoryUsedBytes <= "+maxB+" but got "+(r&&r.memoryUsedBytes));}};}var m={describe:function(n,fn){__st.suites.push(String(n));__st.be.push([]);__st.ae.push([]);try{fn();}finally{__st.suites.pop();__st.be.pop();__st.ae.pop();}},beforeEach:function(fn){__st.be[__st.be.length-1].push(fn);},afterEach:function(fn){__st.ae[__st.ae.length-1].push(fn);},test:function(n,fn,opts){var suite=__st.suites.join(" > "),full=suite?(suite+" > "+n):String(n),flt=globalThis.__slice_test_filter||"";if(flt&&full.indexOf(flt)===-1){__st.skipped++;return;}var iso=!(opts&&opts.isolateState===false),snap=iso?__slice_checkpoint():0,t0=performance.now(),err=null;try{for(var i=0;i<__st.be.length;i++)for(var j=0;j<__st.be[i].length;j++)__st.be[i][j]();var r=fn();if(r&&typeof r.then==="function"){r.then(function(){},function(e){err=String(e&&e.message||e);});}__slice_drain_event_loop();for(var k=__st.ae.length-1;k>=0;k--)for(var l=0;l<__st.ae[k].length;l++)__st.ae[k][l]();}catch(e){err=String(e&&e.message||e);}finally{__slice_clear_timers();if(snap>0){__slice_restore(snap);__st.rollbacks++;}}var dur=Math.max(1,Math.round((performance.now()-t0)*1000));if(!err){__st.passed++;__st.res.push({suite:suite,name:String(n),fullName:full,passed:true,durationUs:dur,checkpointId:snap,restored:snap>0,error:null});}else{__st.failed++;__st.res.push({suite:suite,name:String(n),fullName:full,passed:false,durationUs:dur,checkpointId:snap,restored:snap>0,error:err});}},expect:function(v){return __mkExp(v,false);},summary:function(){return{total:__st.passed+__st.failed+__st.skipped,passed:__st.passed,failed:__st.failed,skipped:__st.skipped,checkpointsRestored:__st.rollbacks,results:__st.res.slice()};},reset:function(){__st.suites=[];__st.be=[[]],__st.ae=[[]];__st.res=[];__st.passed=0;__st.failed=0;__st.skipped=0;__st.rollbacks=0;}};m.it=m.test;m.default=m;m.__esModule=true;__slice_builtins["slice:test"]=__slice_builtins["node:test"]=__slice_builtins["bun:test"]=m;"#,
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
            let tmp = std::env::temp_dir().to_string_lossy().replace('\\', "/");
            Ok(BoaVal::from(js_string!(tmp)))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_os_platform"),
        0,
        NativeFunction::from_fn_ptr(|_this, _args, _context| {
            let is_det = BOA_DET_STATE.with(|s| s.get().is_some());
            let plat = if is_det {
                "linux"
            } else {
                match std::env::consts::OS {
                    "windows" => "win32",
                    "macos" => "darwin",
                    other => other,
                }
            };
            Ok(BoaVal::from(js_string!(plat)))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_os_arch"),
        0,
        NativeFunction::from_fn_ptr(|_this, _args, _context| {
            let is_det = BOA_DET_STATE.with(|s| s.get().is_some());
            let arch = if is_det {
                "x64"
            } else {
                match std::env::consts::ARCH {
                    "x86_64" => "x64",
                    "aarch64" => "arm64",
                    other => other,
                }
            };
            Ok(BoaVal::from(js_string!(arch)))
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
                Some((maj, min, pat)) => {
                    Ok(BoaVal::from(js_string!(format!("{}.{}.{}", maj, min, pat))))
                }
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
                Some((maj, min, pat)) => {
                    Ok(BoaVal::from(js_string!(format!("{}.{}.{}", maj, min, pat))))
                }
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
            if let (Some(va), Some(vb)) = (semver_parse_strict_xyz(&a), semver_parse_strict_xyz(&b))
            {
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
            if args
                .get(1)
                .is_some_and(|v| !v.is_undefined() && !v.is_null())
            {
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
        4,
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
            let headers_json = args
                .get(3)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let extra_headers: Vec<(String, String)> = if headers_json.is_empty() {
                Vec::new()
            } else {
                serde_json::from_str(&headers_json).unwrap_or_default()
            };
            BOA_HTTP_RES.with(|res| {
                *res.borrow_mut() = Some((status, body, content_type, extra_headers));
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
            let saved_ev_loop = save_and_reset_event_loop_state();

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
            restore_event_loop_state(saved_ev_loop);
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
        5,
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
            let redirect_mode = args
                .get(4)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_else(|| "follow".to_string());

            let Some(parts) = parse_outbound_http_url(&url) else {
                let err_json =
                    serde_json::json!({ "__error": format!("Invalid outbound URL: {}", url) })
                        .to_string();
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
                map.get(&method_url_key).or_else(|| map.get(&url)).cloned()
            }) {
                return Ok(BoaVal::from(js_string!(mock_json)));
            }

            // 2. Check deterministic KV cassette (`fetch_cassette:<METHOD>:<URL>`)
            let cassette_key = format!("fetch_cassette:{}:{}", method, url);
            let tenant_id = BOA_ACTIVE_TENANT.with(|t| t.borrow().clone());
            if let Some(cassette_json) = edge_kv_get(&tenant_id, &cassette_key) {
                return Ok(BoaVal::from(js_string!(cassette_json)));
            }

            // 3. Execute real outbound HTTP/1.1 or HTTPS (TLS 1.3/1.2) request with hop-by-hop redirect guard
            let req_headers: Vec<(String, String)> =
                serde_json::from_str(&headers_json).unwrap_or_default();
            match execute_outbound_fetch_with_redirects(
                &url,
                &method,
                &req_headers,
                &body,
                &redirect_mode,
            ) {
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

    // Phase N-2: Event Loop, Timer Wheel (`setTimeout`, `setInterval`), and Concurrent Async `fetch`
    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_timer_set"),
        4,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let Some(cb_obj) = args.first().and_then(|v| v.as_object()) else {
                return Ok(BoaVal::from(0));
            };
            let delay_ms = args
                .get(1)
                .and_then(|v| v.as_number())
                .map(|n| n.max(0.0) as u64)
                .unwrap_or(0);
            let is_interval = args.get(2).and_then(|v| v.as_boolean()).unwrap_or(false);
            let mut extra_args = Vec::new();
            if let Some(arr_obj) = args.get(3).and_then(|v| v.as_object()) {
                if let Ok(len_val) = arr_obj.get(js_string!("length"), context) {
                    let len = len_val.as_number().unwrap_or(0.0) as u32;
                    for i in 0..len.min(8) {
                        if let Ok(elem) = arr_obj.get(i, context) {
                            extra_args.push(elem);
                        }
                    }
                }
            }
            match schedule_timer(cb_obj.clone(), delay_ms, is_interval, extra_args) {
                Ok(id) => Ok(BoaVal::from(id as f64)),
                Err(_) => Ok(BoaVal::from(0)),
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_timer_clear"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, _context| {
            let id = args
                .first()
                .and_then(|v| v.as_number())
                .map(|n| n.max(0.0) as u32)
                .unwrap_or(0);
            cancel_timer(id);
            Ok(BoaVal::undefined())
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_clear_timers"),
        0,
        NativeFunction::from_fn_ptr(|_this, _args, _context| {
            clear_event_loop_state();
            Ok(BoaVal::undefined())
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_drain_event_loop"),
        0,
        NativeFunction::from_fn_ptr(|_this, _args, context| {
            drive_event_loop_to_completion(context);
            Ok(BoaVal::undefined())
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fetch_try_fast"),
        5,
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
            let _redirect_mode = args
                .get(4)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_else(|| "follow".to_string());

            let Some(parts) = parse_outbound_http_url(&url) else {
                let err_json =
                    serde_json::json!({ "__error": format!("Invalid outbound URL: {}", url) })
                        .to_string();
                return Ok(BoaVal::from(js_string!(err_json)));
            };

            if let Err(err_msg) =
                check_boa_outbound_fetch_ocap_and_dlp(&parts, &url, &headers_json, &body)
            {
                let err_json = serde_json::json!({ "__error": err_msg }).to_string();
                return Ok(BoaVal::from(js_string!(err_json)));
            }

            let method_url_key = format!("{} {}", method, url);
            if let Some(mock_json) = BOA_FETCH_MOCKS.with(|m| {
                let map = m.borrow();
                map.get(&method_url_key).or_else(|| map.get(&url)).cloned()
            }) {
                return Ok(BoaVal::from(js_string!(mock_json)));
            }

            let cassette_key = format!("fetch_cassette:{}:{}", method, url);
            let tenant_id = BOA_ACTIVE_TENANT.with(|t| t.borrow().clone());
            if let Some(cassette_json) = edge_kv_get(&tenant_id, &cassette_key) {
                return Ok(BoaVal::from(js_string!(cassette_json)));
            }

            Ok(BoaVal::from(js_string!("")))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fetch_enqueue"),
        8,
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
            let redirect_mode = args
                .get(4)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_else(|| "follow".to_string());
            let Some(resolve_fn) = args.get(5).and_then(|v| v.as_object()) else {
                return Ok(BoaVal::from(0));
            };
            let Some(reject_fn) = args.get(6).and_then(|v| v.as_object()) else {
                return Ok(BoaVal::from(0));
            };
            let signal_obj = args.get(7).and_then(|v| v.as_object());

            let req_headers: Vec<(String, String)> =
                serde_json::from_str(&headers_json).unwrap_or_default();
            let id = enqueue_async_fetch(
                url,
                method,
                req_headers,
                body,
                redirect_mode,
                resolve_fn.clone(),
                reject_fn.clone(),
                signal_obj,
            );
            Ok(BoaVal::from(id as f64))
        }),
    );

    register_wintertc_phase_n3_host_functions(ctx);
}

fn extract_buffer_source_bytes(
    val: &boa_engine::JsValue,
    context: &mut boa_engine::Context,
) -> boa_engine::JsResult<Vec<u8>> {
    use boa_engine::js_string;
    use boa_engine::object::builtins::JsArrayBuffer;

    if let Some(s) = val.as_string() {
        return Ok(s.to_std_string_escaped().into_bytes());
    }
    let Some(obj) = val.as_object() else {
        return Ok(Vec::new());
    };
    if let Ok(ab) = JsArrayBuffer::from_object(obj.clone()) {
        if let Some(data) = ab.data() {
            return Ok(data.to_vec());
        }
        return Ok(Vec::new());
    }
    if let Ok(buf_val) = obj.get(js_string!("buffer"), context) {
        if let Some(buf_obj) = buf_val.as_object() {
            if let Ok(ab) = JsArrayBuffer::from_object(buf_obj) {
                let offset = obj
                    .get(js_string!("byteOffset"), context)
                    .ok()
                    .and_then(|v| v.as_number())
                    .map(|n| n.max(0.0) as usize)
                    .unwrap_or(0);
                let len = obj
                    .get(js_string!("byteLength"), context)
                    .ok()
                    .and_then(|v| v.as_number())
                    .map(|n| n.max(0.0) as usize);
                if let Some(data) = ab.data() {
                    let start = offset.min(data.len());
                    let end = len.map(|l| (start + l).min(data.len())).unwrap_or(data.len());
                    return Ok(data[start..end].to_vec());
                }
            }
        }
    }
    if let Ok(len_val) = obj.get(js_string!("length"), context) {
        if let Some(len) = len_val.as_number().map(|n| n.max(0.0) as usize) {
            let mut out = Vec::with_capacity(len.min(1_048_576));
            for i in 0..len.min(1_048_576) {
                let b = obj
                    .get(i as u32, context)?
                    .as_number()
                    .map(|n| (n as i64 & 0xff) as u8)
                    .unwrap_or(0);
                out.push(b);
            }
            return Ok(out);
        }
    }
    Ok(Vec::new())
}

fn bytes_to_js_array_buffer(
    bytes: &[u8],
    context: &mut boa_engine::Context,
) -> boa_engine::JsResult<boa_engine::JsValue> {
    use boa_engine::object::builtins::{AlignedVec, JsArrayBuffer};
    let block = AlignedVec::from_slice(0, bytes);
    let ab = JsArrayBuffer::from_byte_block(block, context)?;
    Ok(ab.into())
}

fn bytes_to_js_uint8_array(
    bytes: &[u8],
    context: &mut boa_engine::Context,
) -> boa_engine::JsResult<boa_engine::JsValue> {
    use boa_engine::object::builtins::{AlignedVec, JsArrayBuffer, JsUint8Array};
    let block = AlignedVec::from_slice(0, bytes);
    let ab = JsArrayBuffer::from_byte_block(block, context)?;
    let u8_arr = JsUint8Array::from_array_buffer(ab, context)?;
    Ok(u8_arr.into())
}

#[inline]
fn normalize_webcrypto_hash_name(raw: &str) -> Option<&'static str> {
    let trimmed = raw.trim();
    if trimmed.eq_ignore_ascii_case("SHA-1") || trimmed.eq_ignore_ascii_case("SHA1") {
        Some("SHA-1")
    } else if trimmed.eq_ignore_ascii_case("SHA-256") || trimmed.eq_ignore_ascii_case("SHA256") {
        Some("SHA-256")
    } else if trimmed.eq_ignore_ascii_case("SHA-384") || trimmed.eq_ignore_ascii_case("SHA384") {
        Some("SHA-384")
    } else if trimmed.eq_ignore_ascii_case("SHA-512") || trimmed.eq_ignore_ascii_case("SHA512") {
        Some("SHA-512")
    } else {
        None
    }
}

fn compute_webcrypto_hmac(hash_name: &str, key: &[u8], data: &[u8]) -> Option<Vec<u8>> {
    match normalize_webcrypto_hash_name(hash_name)? {
        "SHA-1" => Some(hmac_sha1_bytes(key, data).to_vec()),
        "SHA-256" => Some(hmac_sha256_bytes(key, data).to_vec()),
        "SHA-384" => Some(hmac_sha384_bytes(key, data).to_vec()),
        "SHA-512" => Some(hmac_sha512_bytes(key, data).to_vec()),
        _ => None,
    }
}

fn register_wintertc_phase_n3_host_functions(ctx: &mut boa_engine::Context) {
    use boa_engine::object::builtins::JsArrayBuffer;
    use boa_engine::{js_string, JsNativeError, JsValue as BoaVal, NativeFunction};

    // 1. WhatWG `btoa` & `atob`
    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_btoa"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let s = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|js| js.to_std_string_escaped())
                .unwrap_or_default();
            match whatwg_btoa(&s) {
                Ok(encoded) => Ok(BoaVal::from(js_string!(encoded))),
                Err(msg) => Err(JsNativeError::error().with_message(msg).into()),
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_atob"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let s = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|js| js.to_std_string_escaped())
                .unwrap_or_default();
            match whatwg_atob(&s) {
                Ok(decoded) => Ok(BoaVal::from(js_string!(decoded))),
                Err(msg) => Err(JsNativeError::error().with_message(msg).into()),
            }
        }),
    );

    // 2. Native `TextEncoder` (`__slice_text_encode`, `__slice_text_encode_into`) & `TextDecoder` (`__slice_text_decode`)
    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_text_encode"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let s = match args.first() {
                None => String::new(),
                Some(v) if v.is_undefined() => String::new(),
                Some(v) => v.to_string(context)?.to_std_string_escaped(),
            };
            bytes_to_js_uint8_array(s.as_bytes(), context)
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_text_encode_into"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let s = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|js| js.to_std_string_escaped())
                .unwrap_or_default();
            let Some(dest_obj) = args.get(1).and_then(|v| v.as_object()) else {
                return Err(JsNativeError::typ()
                    .with_message("TextEncoder.encodeInto requires a Uint8Array destination")
                    .into());
            };
            let buf_val = dest_obj.get(js_string!("buffer"), context)?;
            let Some(buf_obj) = buf_val.as_object() else {
                return Err(JsNativeError::typ()
                    .with_message("TextEncoder.encodeInto requires a Uint8Array destination")
                    .into());
            };
            let ab = JsArrayBuffer::from_object(buf_obj)?;
            let offset = dest_obj
                .get(js_string!("byteOffset"), context)
                .ok()
                .and_then(|v| v.as_number())
                .map(|n| n.max(0.0) as usize)
                .unwrap_or(0);
            let byte_len = dest_obj
                .get(js_string!("byteLength"), context)
                .ok()
                .and_then(|v| v.as_number())
                .map(|n| n.max(0.0) as usize)
                .unwrap_or(0);

            let mut read_utf16 = 0usize;
            let mut written_utf8 = 0usize;
            if let Some(mut data_mut) = ab.data_mut() {
                let start = offset.min(data_mut.len());
                let end = (start + byte_len).min(data_mut.len());
                let dest_slice = &mut data_mut[start..end];
                for ch in s.chars() {
                    let need = ch.len_utf8();
                    if written_utf8 + need > dest_slice.len() {
                        break;
                    }
                    ch.encode_utf8(&mut dest_slice[written_utf8..written_utf8 + need]);
                    written_utf8 += need;
                    read_utf16 += ch.len_utf16();
                }
            }

            let res_obj = boa_engine::JsObject::with_object_proto(context.intrinsics());
            let _ = res_obj.set(js_string!("read"), BoaVal::from(read_utf16 as f64), false, context);
            let _ = res_obj.set(
                js_string!("written"),
                BoaVal::from(written_utf8 as f64),
                false,
                context,
            );
            Ok(res_obj.into())
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_text_decode"),
        4,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let Some(first) = args.first() else {
                return Ok(BoaVal::from(js_string!("")));
            };
            if first.is_undefined() || first.is_null() {
                return Ok(BoaVal::from(js_string!("")));
            }
            let bytes = extract_buffer_source_bytes(first, context)?;
            let enc = args
                .get(1)
                .and_then(|v| v.as_string())
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_else(|| "utf-8".to_string());
            let fatal = args.get(2).and_then(|v| v.as_boolean()).unwrap_or(false);
            let ignore_bom = args.get(3).and_then(|v| v.as_boolean()).unwrap_or(false);

            if enc.eq_ignore_ascii_case("latin1") || enc.eq_ignore_ascii_case("iso-8859-1") {
                let mut s = String::with_capacity(bytes.len());
                for &b in &bytes {
                    s.push(b as char);
                }
                return Ok(BoaVal::from(js_string!(s)));
            }
            if enc.eq_ignore_ascii_case("ascii") || enc.eq_ignore_ascii_case("us-ascii") {
                let mut s = String::with_capacity(bytes.len());
                for &b in &bytes {
                    if b > 0x7F {
                        if fatal {
                            return Err(JsNativeError::typ()
                                .with_message("The encoded data was not valid for encoding ascii")
                                .into());
                        }
                        s.push('\u{FFFD}');
                    } else {
                        s.push(b as char);
                    }
                }
                return Ok(BoaVal::from(js_string!(s)));
            }

            let slice = if !ignore_bom && bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
                &bytes[3..]
            } else {
                &bytes[..]
            };
            if fatal {
                match std::str::from_utf8(slice) {
                    Ok(valid) => Ok(BoaVal::from(js_string!(valid))),
                    Err(_) => Err(JsNativeError::typ()
                        .with_message("The encoded data was not valid for encoding utf-8")
                        .into()),
                }
            } else {
                let decoded = String::from_utf8_lossy(slice).into_owned();
                Ok(BoaVal::from(js_string!(decoded)))
            }
        }),
    );

    // 3. WebCrypto `crypto.randomUUID()`, `crypto.getRandomValues()`, and `crypto.subtle`
    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_crypto_random_uuid"),
        0,
        NativeFunction::from_fn_ptr(|_this, _args, _context| {
            let uuid = BOA_DET_STATE.with(|s| {
                if let Some((mut prng, clk)) = s.get() {
                    let u = generate_uuid_v4(Some(&mut prng));
                    s.set(Some((prng, clk)));
                    u
                } else {
                    generate_uuid_v4(None)
                }
            });
            Ok(BoaVal::from(js_string!(uuid)))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_crypto_get_random_values"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let Some(val) = args.first() else {
                return Err(JsNativeError::typ()
                    .with_message("crypto.getRandomValues requires an ArrayBufferView")
                    .into());
            };
            let Some(obj) = val.as_object() else {
                return Err(JsNativeError::typ()
                    .with_message("crypto.getRandomValues requires an ArrayBufferView")
                    .into());
            };
            // Verify integer TypedArray (reject Float32Array, Float64Array, DataView)
            let ctor_name = obj
                .get(js_string!("constructor"), context)
                .ok()
                .and_then(|c| c.as_object())
                .and_then(|co| co.get(js_string!("name"), context).ok())
                .and_then(|n| n.as_string())
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            if !matches!(
                ctor_name.as_str(),
                "Int8Array"
                    | "Uint8Array"
                    | "Uint8ClampedArray"
                    | "Int16Array"
                    | "Uint16Array"
                    | "Int32Array"
                    | "Uint32Array"
                    | "BigInt64Array"
                    | "BigUint64Array"
                    | "Buffer"
            ) {
                return Err(JsNativeError::typ()
                    .with_message(
                        "TypeMismatchError: crypto.getRandomValues requires an integer TypedArray",
                    )
                    .into());
            }

            let buf_val = obj.get(js_string!("buffer"), context)?;
            let Some(buf_obj) = buf_val.as_object() else {
                return Err(JsNativeError::typ()
                    .with_message("crypto.getRandomValues requires a TypedArray")
                    .into());
            };
            let ab = JsArrayBuffer::from_object(buf_obj)?;
            let offset = obj
                .get(js_string!("byteOffset"), context)
                .ok()
                .and_then(|v| v.as_number())
                .map(|n| n.max(0.0) as usize)
                .unwrap_or(0);
            let byte_len = obj
                .get(js_string!("byteLength"), context)
                .ok()
                .and_then(|v| v.as_number())
                .map(|n| n.max(0.0) as usize)
                .unwrap_or(0);

            if byte_len > 65_536 {
                return Err(JsNativeError::error()
                    .with_message(
                        "QuotaExceededError: crypto.getRandomValues byteLength exceeds 65536 bytes",
                    )
                    .into());
            }

            if let Some(mut data_mut) = ab.data_mut() {
                let start = offset.min(data_mut.len());
                let end = (start + byte_len).min(data_mut.len());
                let slice = &mut data_mut[start..end];
                BOA_DET_STATE.with(|s| {
                    if let Some((mut prng, clk)) = s.get() {
                        fill_random_bytes(slice, Some(&mut prng));
                        s.set(Some((prng, clk)));
                    } else {
                        fill_random_bytes(slice, None);
                    }
                });
            }
            Ok(val.clone())
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_crypto_digest"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let algo = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let Some(data_arg) = args.get(1) else {
                return Err(JsNativeError::typ()
                    .with_message("crypto.subtle.digest requires data argument")
                    .into());
            };
            let bytes = extract_buffer_source_bytes(data_arg, context)?;
            let digest: Vec<u8> = match normalize_webcrypto_hash_name(&algo) {
                Some("SHA-1") => sha1_bytes(&bytes).to_vec(),
                Some("SHA-256") => sha256_bytes(&bytes).to_vec(),
                Some("SHA-384") => sha384_bytes(&bytes).to_vec(),
                Some("SHA-512") => sha512_bytes(&bytes).to_vec(),
                _ => {
                    return Err(JsNativeError::error()
                        .with_message(format!(
                            "NotSupportedError: Unsupported digest algorithm '{}'",
                            algo
                        ))
                        .into())
                }
            };
            bytes_to_js_array_buffer(&digest, context)
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_crypto_hmac_sign"),
        3,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let hash_name = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_else(|| "SHA-256".to_string());
            let Some(key_arg) = args.get(1) else {
                return Err(JsNativeError::typ()
                    .with_message("HMAC sign requires key bytes")
                    .into());
            };
            let Some(data_arg) = args.get(2) else {
                return Err(JsNativeError::typ()
                    .with_message("HMAC sign requires data bytes")
                    .into());
            };
            let key_bytes = extract_buffer_source_bytes(key_arg, context)?;
            let data_bytes = extract_buffer_source_bytes(data_arg, context)?;
            let Some(mac) = compute_webcrypto_hmac(&hash_name, &key_bytes, &data_bytes) else {
                return Err(JsNativeError::error()
                    .with_message(format!(
                        "NotSupportedError: Unsupported HMAC hash '{}'",
                        hash_name
                    ))
                    .into());
            };
            bytes_to_js_array_buffer(&mac, context)
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_crypto_hmac_verify"),
        4,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let hash_name = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_else(|| "SHA-256".to_string());
            let Some(key_arg) = args.get(1) else {
                return Ok(BoaVal::from(false));
            };
            let Some(sig_arg) = args.get(2) else {
                return Ok(BoaVal::from(false));
            };
            let Some(data_arg) = args.get(3) else {
                return Ok(BoaVal::from(false));
            };
            let key_bytes = extract_buffer_source_bytes(key_arg, context)?;
            let sig_bytes = extract_buffer_source_bytes(sig_arg, context)?;
            let data_bytes = extract_buffer_source_bytes(data_arg, context)?;
            let Some(expected) = compute_webcrypto_hmac(&hash_name, &key_bytes, &data_bytes) else {
                return Err(JsNativeError::error()
                    .with_message(format!(
                        "NotSupportedError: Unsupported HMAC hash '{}'",
                        hash_name
                    ))
                    .into());
            };
            Ok(BoaVal::from(constant_time_eq(&expected, &sig_bytes)))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_crypto_timing_safe_equal"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let Some(a_arg) = args.first() else {
                return Err(JsNativeError::typ()
                    .with_message("timingSafeEqual requires 2 arguments")
                    .into());
            };
            let Some(b_arg) = args.get(1) else {
                return Err(JsNativeError::typ()
                    .with_message("timingSafeEqual requires 2 arguments")
                    .into());
            };
            let a_bytes = extract_buffer_source_bytes(a_arg, context)?;
            let b_bytes = extract_buffer_source_bytes(b_arg, context)?;
            if a_bytes.len() != b_bytes.len() {
                return Err(JsNativeError::range()
                    .with_message("Input buffers must have the same byte length")
                    .into());
            }
            Ok(BoaVal::from(constant_time_eq(&a_bytes, &b_bytes)))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_bytes_to_hex"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let Some(first) = args.first() else {
                return Ok(BoaVal::from(js_string!("")));
            };
            let bytes = extract_buffer_source_bytes(first, context)?;
            Ok(BoaVal::from(js_string!(bytes_to_hex(&bytes))))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_base64_decode_bytes"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let s = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|js| js.to_std_string_escaped())
                .unwrap_or_default();
            let decoded = base64_decode(&s);
            bytes_to_js_uint8_array(&decoded, context)
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_base64_encode_bytes"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let Some(first) = args.first() else {
                return Ok(BoaVal::from(js_string!("")));
            };
            let url_safe = args.get(1).and_then(|v| v.as_boolean()).unwrap_or(false);
            let bytes = extract_buffer_source_bytes(first, context)?;
            Ok(BoaVal::from(js_string!(base64_encode(&bytes, url_safe))))
        }),
    );
}
