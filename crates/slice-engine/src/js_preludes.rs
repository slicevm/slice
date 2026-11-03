//! WinterTC, WHATWG, Node.js `Buffer`, Event-Loop Microtask/Immediate, and `console.*` Preludes.

pub(crate) const CORE_PRELUDE: &[u8] = br##"globalThis.console={log:__slice_console_log,info:__slice_console_log,warn:__slice_console_log,error:__slice_console_log,debug:__slice_console_log};globalThis.print=__slice_console_log;globalThis.env=function(k){return __slice_env_get(String(k));};globalThis.process={pid:1,env:{NODE_ENV:"production"},cwd:function(){return ".";},get platform(){return __slice_os_platform();},get arch(){return __slice_os_arch();},version:"v24.0.0",versions:{node:"24.0.0"},argv:["slice"]};globalThis.__slice_check_ses=function(){var p=[Object.prototype,Array.prototype,Function.prototype,String.prototype],n=["Object.prototype","Array.prototype","Function.prototype","String.prototype"];for(var i=0;i<4;i++){var ks=Object.keys(p[i]);if(ks.length>0){for(var j=0;j<ks.length;j++)delete p[i][ks[j]];return n[i];}}return "";};globalThis.__slice_sync_process_env=function(load){if(!load){globalThis.process.env={NODE_ENV:"production"};return;}var m=JSON.parse(__slice_env_json());m.NODE_ENV=m.NODE_ENV||"production";globalThis.process.env=m;};var __b2s=function(e){return e==="base64"?__slice_base64_encode(this._s,false):e==="base64url"?__slice_base64_encode(this._s,true):this._s;};globalThis.Buffer={from:function(i,e){var r=typeof i==="string"?i:String(i),s=(e==="base64"||e==="base64url")?__slice_base64_decode(r):r;return{_s:s,length:s.length,toString:__b2s};},isBuffer:function(){return false;},byteLength:function(s){return String(s).length;}};var __orand=Math.random,__onow=Date.now;Math.random=function(){var v=__slice_det_random();return v>=0?v:__orand();};Date.now=function(){var v=__slice_det_now();return v>=0?v:__onow();};globalThis.performance={now:function(){var v=__slice_det_now();return v>=0?(v-1700000000000):(__onow()-1700000000000);}};var __ntQ=[],__qmQ=[],__mtS=false;function __schedMt(){if(__mtS)return;__mtS=true;Promise.resolve().then(__drainMt);}function __drainMt(){while(__ntQ.length>0||__qmQ.length>0){while(__ntQ.length>0){var n=__ntQ.shift();n.f.apply(undefined,n.a);}if(__qmQ.length>0){var q=__qmQ.shift();q();}}__mtS=false;if(__ntQ.length>0||__qmQ.length>0)__schedMt();}var __origThen=Promise.prototype.then;Promise.prototype.then=function(onF,onR){var wF=typeof onF==="function"?function(v){while(__ntQ.length>0){var n=__ntQ.shift();n.f.apply(undefined,n.a);}return onF(v);}:onF;var wR=typeof onR==="function"?function(e){while(__ntQ.length>0){var n=__ntQ.shift();n.f.apply(undefined,n.a);}return onR(e);}:onR;return __origThen.call(this,wF,wR);};globalThis.process.nextTick=function(cb){if(typeof cb!=="function")throw new TypeError("process.nextTick requires a callback function");var a=[];for(var i=1;i<arguments.length;i++)a.push(arguments[i]);__ntQ.push({f:cb,a:a});__schedMt();};globalThis.queueMicrotask=function(cb){if(typeof cb!=="function")throw new TypeError("queueMicrotask requires a callback function");__qmQ.push(cb);__schedMt();};globalThis.setTimeout=function(cb,ms){var a=[];for(var i=2;i<arguments.length;i++)a.push(arguments[i]);return __slice_timer_set(cb,Number(ms)||0,false,a);};globalThis.clearTimeout=function(id){__slice_timer_clear(Number(id)||0);};globalThis.setInterval=function(cb,ms){var a=[];for(var i=2;i<arguments.length;i++)a.push(arguments[i]);return __slice_timer_set(cb,Number(ms)||0,true,a);};globalThis.clearInterval=globalThis.clearTimeout;globalThis.setImmediate=function(cb){if(typeof cb!=="function")throw new TypeError("setImmediate requires a callback function");var a=[];for(var i=1;i<arguments.length;i++)a.push(arguments[i]);return __slice_immediate_set(cb,a);};globalThis.clearImmediate=globalThis.clearTimeout;function AbortSignal(){this.aborted=false;this.reason=undefined;this.onabort=null;this._ls=[];}AbortSignal.prototype.addEventListener=function(t,fn){if(t==="abort"&&typeof fn==="function")this._ls.push(fn);};AbortSignal.prototype.removeEventListener=function(t,fn){if(t==="abort"){var nx=[];for(var i=0;i<this._ls.length;i++)if(this._ls[i]!==fn)nx.push(this._ls[i]);this._ls=nx;}};AbortSignal.prototype.throwIfAborted=function(){if(this.aborted)throw this.reason;};AbortSignal.prototype._fire=function(r){if(this.aborted)return;this.aborted=true;this.reason=r!==undefined?r:new Error("AbortError: The operation was aborted");var ev={type:"abort",target:this};if(typeof this.onabort==="function")try{this.onabort(ev);}catch(_e){}var ls=this._ls.slice();this._ls=[];for(var i=0;i<ls.length;i++)try{ls[i](ev);}catch(_e2){}};AbortSignal.abort=function(r){var s=new AbortSignal();s._fire(r!==undefined?r:new Error("AbortError: The operation was aborted"));return s;};AbortSignal.timeout=function(ms){var s=new AbortSignal();setTimeout(function(){s._fire(new Error("TimeoutError: The operation was aborted due to timeout"));},Number(ms)||0);return s;};function AbortController(){this.signal=new AbortSignal();}AbortController.prototype.abort=function(r){this.signal._fire(r);};globalThis.AbortSignal=AbortSignal;globalThis.AbortController=AbortController;function __fmtSse(ev){if(ev===undefined||ev===null)return "";if(typeof ev==="string"){if(ev.indexOf("\n")===-1)return "data: "+ev+"\n\n";var ls=ev.split("\n"),o="";for(var i=0;i<ls.length;i++)o+="data: "+ls[i]+"\n";return o+"\n";}var s="";if(ev.id!==undefined&&ev.id!==null)s+="id: "+String(ev.id)+"\n";if(ev.event!==undefined&&ev.event!==null)s+="event: "+String(ev.event)+"\n";if(ev.retry!==undefined&&ev.retry!==null)s+="retry: "+Number(ev.retry)+"\n";var d=ev.data!==undefined?(typeof ev.data==="string"?ev.data:JSON.stringify(ev.data)):JSON.stringify(ev);if(d.indexOf("\n")===-1)return s+"data: "+d+"\n\n";var dl=d.split("\n");for(var j=0;j<dl.length;j++)s+="data: "+dl[j]+"\n";return s+"\n";}function __colSse(src){var out="";if(Array.isArray(src)){for(var i=0;i<src.length;i++)out+=__fmtSse(src[i]);}else if(typeof src==="function"){var r=src(function(ev){out+=__fmtSse(ev);});if(r&&typeof r.next==="function"){var st=r.next();while(!st.done){out+=__fmtSse(st.value);st=r.next();}}else if(Array.isArray(r)){for(var j=0;j<r.length;j++)out+=__fmtSse(r[j]);}}else if(src&&typeof src.next==="function"){var s2=src.next();while(!s2.done){out+=__fmtSse(s2.value);s2=src.next();}}else if(src!==undefined&&src!==null){out+=__fmtSse(src);}return out;}function ReadableStream(src){var chunks=[],closed=false;this._chunks=chunks;if(src&&typeof src.start==="function"){src.start({enqueue:function(c){if(!closed)chunks.push(typeof c==="string"?c:String(c));},close:function(){closed=true;}});}}ReadableStream.prototype._toText=function(){return this._chunks.join("");};ReadableStream.prototype.getReader=function(){var cs=this._chunks,idx=0;return{read:function(){if(idx<cs.length)return Promise.resolve({value:cs[idx++],done:false});return Promise.resolve({value:undefined,done:true});},releaseLock:function(){}};};globalThis.ReadableStream=ReadableStream;function __doFetch(input,init){var req=new Request(input,init),sig=(init&&init.signal)||req.signal||null;if(sig&&sig.aborted)throw(sig.reason||new Error("AbortError: The operation was aborted"));var raw=JSON.parse(__slice_fetch_sync(req.url,req.method,JSON.stringify(req.headers.entries()),req._body,req.redirect||"follow"));if(raw.__error)throw new Error(raw.__error);var r=new Response(raw.body,{status:raw.status,statusText:raw.statusText,headers:raw.headers});r.url=raw.finalUrl||req.url;r.redirected=Boolean(raw.redirected);return r;}globalThis.fetch=function(input,init){try{var req=new Request(input,init),sig=(init&&init.signal)||req.signal||null;if(sig&&sig.aborted)return Promise.reject(sig.reason||new Error("AbortError: The operation was aborted"));var hdrs=JSON.stringify(req.headers.entries()),fast=__slice_fetch_try_fast(req.url,req.method,hdrs,req._body,req.redirect||"follow");if(fast!==""){var raw=JSON.parse(fast);if(raw.__error)return Promise.reject(new Error(raw.__error));var r=new Response(raw.body,{status:raw.status,statusText:raw.statusText,headers:raw.headers});r.url=raw.finalUrl||req.url;r.redirected=Boolean(raw.redirected);return Promise.resolve(r);}return new Promise(function(res,rej){var settled=false;if(sig&&typeof sig.addEventListener==="function"){sig.addEventListener("abort",function(){if(!settled){settled=true;rej(sig.reason||new Error("AbortError: The operation was aborted"));}});}__slice_fetch_enqueue(req.url,req.method,hdrs,req._body,req.redirect||"follow",function(rawStr){if(settled)return;settled=true;var raw2=JSON.parse(rawStr);if(raw2.__error){rej(new Error(raw2.__error));return;}var r2=new Response(raw2.body,{status:raw2.status,statusText:raw2.statusText,headers:raw2.headers});r2.url=raw2.finalUrl||req.url;r2.redirected=Boolean(raw2.redirected);res(r2);},function(errReason){if(settled)return;settled=true;rej(errReason instanceof Error?errReason:new Error(String(errReason)));},sig);});}catch(e){return Promise.reject(e);}};function __tMod(){return __slice_builtins["slice:test"]||__slice_load_builtin("slice:test");}globalThis.describe=function(n,f){return __tMod().describe(n,f);};globalThis.test=globalThis.it=function(n,f,o){return __tMod().test(n,f,o);};globalThis.beforeEach=function(f){return __tMod().beforeEach(f);};globalThis.afterEach=function(f){return __tMod().afterEach(f);};globalThis.expect=function(v){return __tMod().expect(v);};globalThis.__slice_wu=[];globalThis.Slice={request:function(){var t=__slice_http_tenant();return{method:__slice_http_method(),path:__slice_http_path(),query:__slice_http_query(),tenant:t,tenantId:t,body:__slice_http_body(),header:__slice_http_header};},respond:function(s,b,ct){return __slice_http_respond(s,typeof b==="string"?b:JSON.stringify(b),ct||"application/json","");},streamSSE:function(src,init){var r=Response.sse(src,init);__slice_http_respond(r.status,r._body,r._ct,"");return r;},fetchSync:__doFetch,mockFetch:function(a,b,c){var m="*",u=String(a),o=b||{};if(c!==undefined){m=String(a).toUpperCase();u=String(b);o=c||{};}var h=new Headers(o.headers),bd=o.body===undefined||o.body===null?"":(typeof o.body==="string"?o.body:JSON.stringify(o.body));if(typeof o.body==="object"&&o.body!==null&&!h._m["content-type"])h._m["content-type"]="application/json";__slice_mock_fetch(m==="*"?u:(m+" "+u),JSON.stringify({status:o.status||200,statusText:o.statusText||"OK",headers:h.entries(),body:bd}));},waitUntil:function(f){__slice_wu.push(f);__slice_mark_wait_until();},kv:{get:function(k){return __slice_kv_get(String(k));},set:function(k,v,ttl){return __slice_kv_set(String(k),typeof v==="string"?v:JSON.stringify(v),ttl||0);},del:function(k){return __slice_kv_del(String(k));},incr:function(k,d){return __slice_kv_incr(String(k),d===undefined?1:Number(d));}},checkpoint:function(){return __slice_checkpoint();},restore:function(id){return __slice_restore(Number(id));},evalSandbox:function(code,opts){return JSON.parse(__slice_eval_sandbox(String(code),opts?JSON.stringify(opts):"{}"));},testSummary:function(){return __tMod().summary();},resetTests:function(){return __tMod().reset();}};function __uspD(s){if(s.indexOf("+")===-1&&s.indexOf("%")===-1)return s;var r=s.indexOf("+")===-1?s:s.split("+").join(" ");if(r.indexOf("%")===-1)return r;try{return decodeURIComponent(r);}catch(_e){return r;}}function __uspE(s){var str=String(s),e=encodeURIComponent(str);return str.indexOf(" ")===-1?e:e.split("%20").join("+");}function URLSearchParams(init,u){this._p=[];this._u=u||null;if(!init)return;if(typeof init==="string"){var q=init.charCodeAt(0)===63?init.slice(1):init;if(q.length>0){var ps=q.split("&");for(var i=0;i<ps.length;i++){var sg=ps[i];if(!sg)continue;var eq=sg.indexOf("=");if(eq===-1)this._p.push([__uspD(sg),""]);else this._p.push([__uspD(sg.slice(0,eq)),__uspD(sg.slice(eq+1))]);}}}else if(Array.isArray(init)){for(var j=0;j<init.length;j++){var pr=init[j];if(pr&&pr.length>=2)this._p.push([String(pr[0]),String(pr[1])]);}}else if(typeof init==="object"){if(Array.isArray(init._p)){for(var m=0;m<init._p.length;m++)this._p.push([init._p[m][0],init._p[m][1]]);}else{var ks=Object.keys(init);for(var k=0;k<ks.length;k++)this._p.push([ks[k],String(init[ks[k]])]);}}}URLSearchParams.prototype._sync=function(){if(this._u)this._u._syncSearch();};URLSearchParams.prototype.get=function(n){var k=String(n);for(var i=0;i<this._p.length;i++)if(this._p[i][0]===k)return this._p[i][1];return null;};URLSearchParams.prototype.getAll=function(n){var k=String(n),r=[];for(var i=0;i<this._p.length;i++)if(this._p[i][0]===k)r.push(this._p[i][1]);return r;};URLSearchParams.prototype.has=function(n){var k=String(n);for(var i=0;i<this._p.length;i++)if(this._p[i][0]===k)return true;return false;};URLSearchParams.prototype.set=function(n,v){var k=String(n),s=String(v),p=this._p,len=p.length,fi=-1;for(var i=0;i<len;i++){if(p[i][0]===k){if(fi===-1){p[i][1]=s;fi=i;}else{p.splice(i,1);i--;len--;}}}if(fi===-1)p.push([k,s]);if(this._u)this._u._syncSearch();};URLSearchParams.prototype.append=function(n,v){this._p.push([String(n),String(v)]);if(this._u)this._u._syncSearch();};URLSearchParams.prototype.delete=function(n){var k=String(n),nx=[];for(var i=0;i<this._p.length;i++)if(this._p[i][0]!==k)nx.push(this._p[i]);this._p=nx;if(this._u)this._u._syncSearch();};URLSearchParams.prototype.sort=function(){this._p.sort(function(a,b){return a[0]<b[0]?-1:(a[0]>b[0]?1:0);});if(this._u)this._u._syncSearch();};URLSearchParams.prototype.forEach=function(cb,ta){for(var i=0;i<this._p.length;i++)cb.call(ta,this._p[i][1],this._p[i][0],this);};URLSearchParams.prototype.entries=function(){return this._p.slice();};URLSearchParams.prototype.keys=function(){var r=[];for(var i=0;i<this._p.length;i++)r.push(this._p[i][0]);return r;};URLSearchParams.prototype.values=function(){var r=[];for(var i=0;i<this._p.length;i++)r.push(this._p[i][1]);return r;};URLSearchParams.prototype.toString=function(){var len=this._p.length;if(len===0)return "";var s="";for(var i=0;i<len;i++){if(i>0)s+="&";s+=__uspE(this._p[i][0])+"="+__uspE(this._p[i][1]);}return s;};if(typeof Symbol!=="undefined"&&Symbol.iterator)URLSearchParams.prototype[Symbol.iterator]=function(){return this._p[Symbol.iterator]();};Object.defineProperty(URLSearchParams.prototype,"size",{get:function(){return this._p.length;}});function __normP(p){if(!p||p==="/")return "/";if(p.indexOf("/.")===-1&&p.indexOf("//")===-1)return p;var segs=p.split("/"),out=[];for(var i=0;i<segs.length;i++){var s=segs[i];if(!s||s===".")continue;if(s==="..")out.pop();else out.push(s);}return "/"+out.join("/")+(p.length>1&&p.charCodeAt(p.length-1)===47&&out.length>0?"/":"");}function URL(input,base){var raw=input instanceof URL?input.href:String(input).trim();if(base!==undefined&&raw.indexOf("https://")!==0&&raw.indexOf("http://")!==0&&raw.indexOf("://")===-1){var b=base instanceof URL?base:new URL(String(base));if(raw.indexOf("//")===0)raw=b.protocol+raw;else if(raw.charCodeAt(0)===47)raw=b.origin+raw;else if(raw.charCodeAt(0)===63)raw=b.origin+b.pathname+raw;else if(raw.charCodeAt(0)===35)raw=b.origin+b.pathname+b.search+raw;else{var dir=b.pathname.slice(0,b.pathname.lastIndexOf("/")+1)||"/";raw=b.origin+dir+raw;}}var hi=raw.indexOf("#"),hash="";if(hi!==-1){hash=raw.slice(hi);raw=raw.slice(0,hi);}var qi=raw.indexOf("?"),search="";if(qi!==-1){search=raw.slice(qi);raw=raw.slice(0,qi);}var si=raw.indexOf("://");if(si===-1)throw new TypeError("Invalid URL: "+input);this.protocol=raw.slice(0,si+1).toLowerCase();var rest=raw.slice(si+3),pi=rest.indexOf("/"),authHost=pi===-1?rest:rest.slice(0,pi),pathRaw=pi===-1?"/":rest.slice(pi);var ai=authHost.indexOf("@"),hostPort=authHost;this.username="";this.password="";if(ai!==-1){var up=authHost.slice(0,ai),ci=up.indexOf(":");this.username=ci===-1?up:up.slice(0,ci);this.password=ci===-1?"":up.slice(ci+1);hostPort=authHost.slice(ai+1);}var pci=hostPort.lastIndexOf(":");if(pci!==-1&&hostPort.indexOf("]")<pci){this.hostname=hostPort.slice(0,pci).toLowerCase();var prt=hostPort.slice(pci+1);if((this.protocol==="http:"&&prt==="80")||(this.protocol==="https:"&&prt==="443"))prt="";this.port=prt;}else{this.hostname=hostPort.toLowerCase();this.port="";}this.host=this.port?this.hostname+":"+this.port:this.hostname;this.origin=this.protocol+"//"+this.host;this.pathname=__normP(pathRaw);this.hash=hash==="#"?"":hash;this.searchParams=new URLSearchParams(search,this);this._syncSearch();}URL.prototype._syncSearch=function(){if(this.searchParams._p.length===0){this.search="";this.href=this.origin+this.pathname+this.hash;return;}var qs=this.searchParams.toString();this.search=qs?"?"+qs:"";this.href=this.origin+this.pathname+this.search+this.hash;};URL.prototype.toString=URL.prototype.toJSON=function(){return this.href;};function Headers(init){if(init==="__SLICE_LIVE_REQ__"){this._live=true;this._m=null;return;}this._live=false;this._m=Object.create(null);if(!init)return;if(init instanceof Headers){if(init._m){var ik=Object.keys(init._m);for(var n=0;n<ik.length;n++)this._m[ik[n]]=init._m[ik[n]];}else{var es=init.entries();for(var i=0;i<es.length;i++)this._m[es[i][0]]=es[i][1];}}else if(Array.isArray(init)){for(var j=0;j<init.length;j++)if(init[j]&&init[j].length>=2)this.append(init[j][0],init[j][1]);}else if(typeof init==="object"){var ks=Object.keys(init);for(var k=0;k<ks.length;k++)if(init[ks[k]]!==undefined)this._m[ks[k].toLowerCase()]=String(init[ks[k]]);}}Headers.prototype._ensure=function(){if(this._live&&!this._m){this._m=Object.create(null);var raw=JSON.parse(__slice_http_headers_all());for(var i=0;i<raw.length;i++){var k=raw[i][0].toLowerCase();this._m[k]=this._m[k]?this._m[k]+", "+raw[i][1]:raw[i][1];}this._live=false;}};Headers.prototype.get=function(n){var k=String(n).toLowerCase();if(this._m&&k in this._m)return this._m[k];if(this._live)return __slice_http_header(k);return null;};Headers.prototype.has=function(n){return this.get(n)!==null;};Headers.prototype.set=function(n,v){this._ensure();this._m[String(n).toLowerCase()]=String(v);};Headers.prototype.append=function(n,v){this._ensure();var k=String(n).toLowerCase(),s=String(v);this._m[k]=k in this._m?this._m[k]+", "+s:s;};Headers.prototype.delete=function(n){this._ensure();delete this._m[String(n).toLowerCase()];};Headers.prototype.entries=function(){this._ensure();var ks=Object.keys(this._m).sort(),r=[];for(var i=0;i<ks.length;i++)r.push([ks[i],this._m[ks[i]]]);return r;};Headers.prototype.keys=function(){this._ensure();return Object.keys(this._m).sort();};Headers.prototype.values=function(){var es=this.entries(),r=[];for(var i=0;i<es.length;i++)r.push(es[i][1]);return r;};Headers.prototype.forEach=function(cb,ta){var es=this.entries();for(var i=0;i<es.length;i++)cb.call(ta,es[i][1],es[i][0],this);};if(typeof Symbol!=="undefined"&&Symbol.iterator)Headers.prototype[Symbol.iterator]=function(){return this.entries()[Symbol.iterator]();};function Request(input,init){if(input==="__SLICE_LIVE_REQ__"){this.method=__slice_http_method()||"GET";var h=__slice_http_header("host")||"localhost",p=__slice_http_path()||"/",q=__slice_http_query();this.url="http://"+h+p+(q?"?"+q:"");this.headers=new Headers("__SLICE_LIVE_REQ__");this._body=__slice_http_body()||"";this.redirect="follow";this.signal=null;this.bodyUsed=false;return;}var o=init||{},isReq=input instanceof Request;this.url=isReq?input.url:(input instanceof URL?input.href:String(input));this.method=o.method?String(o.method).toUpperCase():(isReq?input.method:"GET");this.headers=o.headers instanceof Headers?o.headers:new Headers(o.headers||(isReq?input.headers:undefined));this._body=o.body!==undefined&&o.body!==null?(typeof o.body==="string"?o.body:JSON.stringify(o.body)):(isReq?input._body:"");this.redirect=o.redirect?String(o.redirect):(isReq&&input.redirect?input.redirect:"follow");this.signal=o.signal||(isReq&&input.signal?input.signal:null);this.bodyUsed=false;}Request.prototype.text=function(){this.bodyUsed=true;return Promise.resolve(this._body);};Request.prototype.json=function(){this.bodyUsed=true;try{return Promise.resolve(JSON.parse(this._body));}catch(e){return Promise.reject(e);}};Request.prototype.textSync=function(){this.bodyUsed=true;return this._body;};Request.prototype.jsonSync=function(){this.bodyUsed=true;return JSON.parse(this._body);};Request.prototype.clone=function(){return new Request(this.url,{method:this.method,headers:new Headers(this.headers),body:this._body,redirect:this.redirect,signal:this.signal});};function Response(body,init){var o=init||{};this.status=o.status!==undefined?Number(o.status):200;this.statusText=o.statusText!==undefined?String(o.statusText):(this.status===200?"OK":"");this.ok=this.status>=200&&this.status<300;this.headers=o.headers instanceof Headers?o.headers:new Headers(o.headers);this._data=undefined;this._body=body===undefined||body===null?"":(body instanceof ReadableStream?body._toText():(typeof body==="string"?body:JSON.stringify(body)));this._ct=(this.headers._m&&this.headers._m["content-type"])||"text/plain;charset=UTF-8";this._isSliceRes=true;this.bodyUsed=false;}Response.prototype.text=function(){this.bodyUsed=true;if(this._data!==undefined&&!this._body)this._body=JSON.stringify(this._data);return Promise.resolve(this._body);};Response.prototype.json=function(){this.bodyUsed=true;if(this._data!==undefined)return Promise.resolve(this._data);try{return Promise.resolve(JSON.parse(this._body));}catch(e){return Promise.reject(e);}};Response.prototype.textSync=function(){this.bodyUsed=true;if(this._data!==undefined&&!this._body)this._body=JSON.stringify(this._data);return this._body;};Response.prototype.jsonSync=function(){this.bodyUsed=true;if(this._data!==undefined)return this._data;return JSON.parse(this._body);};Response.prototype.clone=function(){if(this._data!==undefined&&!this._body)this._body=JSON.stringify(this._data);return new Response(this._body,{status:this.status,statusText:this.statusText,headers:new Headers(this.headers)});};Response.json=function(data,init){var o=init?(typeof init==="number"?{status:init}:init):{},h=o.headers instanceof Headers?o.headers:new Headers(o.headers);if(!h._m["content-type"])h._m["content-type"]="application/json";var r=new Response(null,{status:o.status||200,statusText:o.statusText,headers:h});r._data=data;r._ct=h._m["content-type"];return r;};Response.sse=function(src,init){var o=init?(typeof init==="number"?{status:init}:init):{},h=o.headers instanceof Headers?o.headers:new Headers(o.headers);if(!h._m["content-type"])h._m["content-type"]="text/event-stream; charset=utf-8";if(!h._m["cache-control"])h._m["cache-control"]="no-cache";var r=new Response(__colSse(src),{status:o.status||200,statusText:o.statusText,headers:h});r._ct=h._m["content-type"];return r;};Response.redirect=function(url,status){var st=status||302;return new Response("",{status:st,headers:{location:String(url)}});};function __resHdrJson(h){if(!h||!h._m)return "";var ks=Object.keys(h._m),out=[];for(var i=0;i<ks.length;i++){var k=ks[i];if(k!=="content-type"&&k!=="content-length")out.push([k,h._m[k]]);}return out.length>0?JSON.stringify(out):"";}globalThis.URLSearchParams=URLSearchParams;globalThis.URL=URL;globalThis.Headers=Headers;globalThis.Request=Request;globalThis.Response=Response;var __slice_web_ctx={waitUntil:Slice.waitUntil,passThroughOnException:function(){}};globalThis.__slice_make_web_request=function(){return new Request("__SLICE_LIVE_REQ__");};globalThis.__slice_dispatch_fetch=function(h){if(!__slice_http_is_active())return;var fn=typeof h.fetch==="function"?h.fetch:(typeof h==="function"?h:null);if(!fn)return;var res=fn.call(h,new Request("__SLICE_LIVE_REQ__"),globalThis.process.env,__slice_web_ctx);if(res){if(res._isSliceRes){var b=res._body||(res._data!==undefined?JSON.stringify(res._data):"");__slice_http_respond(res.status,b,res._ct,__resHdrJson(res.headers));}else if(typeof res.then==="function")res.then(function(r){if(r&&r._isSliceRes){var rb=r._body||(r._data!==undefined?JSON.stringify(r._data):"");__slice_http_respond(r.status,rb,r._ct,__resHdrJson(r.headers));}});}};globalThis.__slice_builtins=Object.create(null);globalThis.__slice_cjs_require=function(id){return __slice_builtins[id]||__slice_load_builtin(id);};globalThis.require=__slice_cjs_require;"##;

pub(crate) const WINTERTC_PHASE_N3_PRELUDE: &[u8] = br##"(function(){
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

var __U8 = globalThis.Uint8Array;
var __u8Proto = __U8.prototype;

function __wrapBuf(u8) {
  Object.setPrototypeOf(u8, Buffer.prototype);
  return u8;
}

function __utf8ByteLen(str) {
  var s = String(str), len = s.length, bytes = 0;
  for (var i = 0; i < len; i++) {
    var c = s.charCodeAt(i);
    if (c < 0x80) bytes += 1;
    else if (c < 0x800) bytes += 2;
    else if (c >= 0xD800 && c <= 0xDBFF && i + 1 < len) {
      var c2 = s.charCodeAt(i + 1);
      if (c2 >= 0xDC00 && c2 <= 0xDFFF) { bytes += 4; i++; }
      else bytes += 3;
    } else bytes += 3;
  }
  return bytes;
}

function Buffer(arg, encOrOffset, length) {
  if (typeof arg === "number") return Buffer.alloc(arg);
  return Buffer.from(arg, encOrOffset, length);
}
Object.setPrototypeOf(Buffer.prototype, __u8Proto);
Object.setPrototypeOf(Buffer, __U8);
Buffer.prototype._isBuffer = true;
Buffer.poolSize = 8192;

Buffer.isBuffer = function(obj) {
  return Boolean(obj && obj._isBuffer === true);
};

Buffer.isEncoding = function(enc) {
  if (typeof enc !== "string") return false;
  var e = enc.toLowerCase();
  return e === "utf8" || e === "utf-8" || e === "hex" || e === "base64" || e === "base64url" ||
         e === "latin1" || e === "binary" || e === "ascii" || e === "ucs2" || e === "ucs-2" ||
         e === "utf16le" || e === "utf-16le";
};

Buffer.from = function(value, encOrOffset, length) {
  if (typeof value === "string") {
    var e = encOrOffset ? String(encOrOffset).toLowerCase() : "utf8";
    if (e === "base64" || e === "base64url") {
      return __wrapBuf(__slice_base64_decode_bytes(value));
    }
    if (e === "hex") {
      return __wrapBuf(__slice_hex_decode(value));
    }
    if (e === "latin1" || e === "binary" || e === "ascii") {
      var n = value.length, u = new __U8(n);
      var mask = e === "ascii" ? 0x7f : 0xff;
      for (var i = 0; i < n; i++) u[i] = value.charCodeAt(i) & mask;
      return __wrapBuf(u);
    }
    return __wrapBuf(__slice_text_encode(value));
  }
  if (value instanceof ArrayBuffer) {
    var off = encOrOffset !== undefined ? Math.max(0, Number(encOrOffset) | 0) : 0;
    var maxLen = Math.max(0, value.byteLength - off);
    var len2 = length !== undefined ? Math.min(maxLen, Math.max(0, Number(length) | 0)) : maxLen;
    return __wrapBuf(new __U8(value, off, len2));
  }
  if (value instanceof __U8) {
    return __wrapBuf(new __U8(value));
  }
  if (value && ArrayBuffer.isView(value)) {
    return __wrapBuf(new __U8(value.buffer.slice(value.byteOffset, value.byteOffset + value.byteLength)));
  }
  if (Array.isArray(value) || (value && typeof value.length === "number")) {
    return __wrapBuf(new __U8(value));
  }
  if (value && value.type === "Buffer" && Array.isArray(value.data)) {
    return __wrapBuf(new __U8(value.data));
  }
  throw new TypeError("The first argument must be of type string, Buffer, ArrayBuffer, Array, or Array-like Object");
};

Buffer.alloc = function(size, fill, encoding) {
  var n = Math.max(0, Number(size) | 0);
  var b = __wrapBuf(new __U8(n));
  if (fill !== undefined && fill !== 0 && n > 0) {
    b.fill(fill, 0, n, encoding);
  }
  return b;
};

Buffer.allocUnsafe = function(size) {
  return Buffer.alloc(size);
};

Buffer.allocUnsafeSlow = function(size) {
  return Buffer.alloc(size);
};

Buffer.byteLength = function(input, encoding) {
  if (typeof input !== "string") {
    if (input && typeof input.byteLength === "number") return input.byteLength;
    if (input && typeof input.length === "number") return input.length;
    input = String(input);
  }
  var e = encoding ? String(encoding).toLowerCase() : "utf8";
  if (e === "utf8" || e === "utf-8") return __utf8ByteLen(input);
  if (e === "hex") return input.length >>> 1;
  if (e === "base64" || e === "base64url") return __slice_base64_decode_bytes(input).byteLength;
  if (e === "latin1" || e === "binary" || e === "ascii") return input.length;
  return __utf8ByteLen(input);
};

Buffer.concat = function(list, totalLength) {
  if (!Array.isArray(list)) throw new TypeError("The \"list\" argument must be an instance of Array");
  if (list.length === 0) return Buffer.alloc(0);
  var total = 0;
  if (totalLength !== undefined) {
    total = Math.max(0, Number(totalLength) | 0);
  } else {
    for (var i = 0; i < list.length; i++) total += list[i].length;
  }
  var out = Buffer.alloc(total);
  var pos = 0;
  for (var j = 0; j < list.length; j++) {
    var chunk = list[j];
    if (!chunk || chunk.length === 0) continue;
    var rem = total - pos;
    if (rem <= 0) break;
    if (chunk.length <= rem) {
      out.set(chunk, pos);
      pos += chunk.length;
    } else {
      out.set(__u8Proto.subarray.call(chunk, 0, rem), pos);
      pos += rem;
    }
  }
  return out;
};

Buffer.compare = function(a, b) {
  if (!(a instanceof __U8) || !(b instanceof __U8)) {
    throw new TypeError("The \"buf1\", \"buf2\" arguments must be Uint8Array or Buffer");
  }
  var len = Math.min(a.length, b.length);
  for (var i = 0; i < len; i++) {
    if (a[i] !== b[i]) return a[i] < b[i] ? -1 : 1;
  }
  return a.length === b.length ? 0 : (a.length < b.length ? -1 : 1);
};

Buffer.prototype.toString = function(encoding, start, end) {
  var len = this.length;
  if (len === 0) return "";
  var s = start === undefined ? 0 : Math.max(0, Number(start) | 0);
  var e = end === undefined ? len : Math.min(len, Number(end) | 0);
  if (s >= e) return "";
  var view = (s === 0 && e === len) ? this : __u8Proto.subarray.call(this, s, e);
  var enc = encoding ? String(encoding).toLowerCase() : "utf8";
  if (enc === "hex") return __slice_bytes_to_hex(view);
  if (enc === "base64") return __slice_base64_encode_bytes(view, false);
  if (enc === "base64url") return __slice_base64_encode_bytes(view, true);
  if (enc === "latin1" || enc === "binary") return __slice_text_decode(view, "latin1", false, true);
  if (enc === "ascii") {
    var out = "";
    for (var i = 0; i < view.length; i++) out += String.fromCharCode(view[i] & 0x7f);
    return out;
  }
  return __slice_text_decode(view, "utf-8", false, true);
};

Buffer.prototype.subarray = function(start, end) {
  return __wrapBuf(__u8Proto.subarray.call(this, start, end));
};

Buffer.prototype.slice = function(start, end) {
  return __wrapBuf(__u8Proto.subarray.call(this, start, end));
};

Buffer.prototype.equals = function(other) {
  if (!(other instanceof __U8)) throw new TypeError("The \"otherBuffer\" argument must be an instance of Buffer or Uint8Array");
  return Buffer.compare(this, other) === 0;
};

Buffer.prototype.compare = function(target, targetStart, targetEnd, sourceStart, sourceEnd) {
  if (!(target instanceof __U8)) throw new TypeError("The \"target\" argument must be an instance of Buffer or Uint8Array");
  var ts = targetStart === undefined ? 0 : Math.max(0, Number(targetStart) | 0);
  var te = targetEnd === undefined ? target.length : Math.min(target.length, Number(targetEnd) | 0);
  var ss = sourceStart === undefined ? 0 : Math.max(0, Number(sourceStart) | 0);
  var se = sourceEnd === undefined ? this.length : Math.min(this.length, Number(sourceEnd) | 0);
  var a = (ss === 0 && se === this.length) ? this : __u8Proto.subarray.call(this, ss, se);
  var b = (ts === 0 && te === target.length) ? target : __u8Proto.subarray.call(target, ts, te);
  return Buffer.compare(a, b);
};

Buffer.prototype.copy = function(target, targetStart, sourceStart, sourceEnd) {
  if (!(target instanceof __U8)) throw new TypeError("The \"target\" argument must be a Buffer or Uint8Array");
  var ts = targetStart === undefined ? 0 : Math.max(0, Number(targetStart) | 0);
  var ss = sourceStart === undefined ? 0 : Math.max(0, Number(sourceStart) | 0);
  var se = sourceEnd === undefined ? this.length : Math.min(this.length, Number(sourceEnd) | 0);
  if (ss >= se || ts >= target.length) return 0;
  var count = Math.min(se - ss, target.length - ts);
  target.set(__u8Proto.subarray.call(this, ss, ss + count), ts);
  return count;
};

Buffer.prototype.fill = function(val, offset, end, encoding) {
  var s = offset === undefined ? 0 : Math.max(0, Number(offset) | 0);
  var e = end === undefined ? this.length : Math.min(this.length, Number(end) | 0);
  if (s >= e) return this;
  if (typeof val === "number") {
    __u8Proto.fill.call(this, val & 0xff, s, e);
    return this;
  }
  var pat = typeof val === "string" ? Buffer.from(val, encoding) : (val instanceof __U8 ? val : Buffer.from([Number(val) & 0xff]));
  if (pat.length === 0) {
    __u8Proto.fill.call(this, 0, s, e);
    return this;
  }
  for (var i = s; i < e; i++) {
    this[i] = pat[(i - s) % pat.length];
  }
  return this;
};

Buffer.prototype.write = function(str, offset, length, encoding) {
  var off = 0, len = this.length, enc = "utf8";
  if (typeof offset === "string") {
    enc = offset;
  } else if (typeof offset === "number") {
    off = offset | 0;
    if (typeof length === "string") enc = length;
    else if (typeof length === "number") {
      len = length | 0;
      if (typeof encoding === "string") enc = encoding;
    }
  }
  var maxAvail = Math.max(0, this.length - off);
  var maxWrite = Math.min(maxAvail, Math.max(0, len));
  if (maxWrite === 0) return 0;
  var src = Buffer.from(String(str), enc);
  var count = Math.min(src.length, maxWrite);
  this.set(__u8Proto.subarray.call(src, 0, count), off);
  return count;
};

Buffer.prototype.toJSON = function() {
  var d = new Array(this.length);
  for (var i = 0; i < this.length; i++) d[i] = this[i];
  return { type: "Buffer", data: d };
};

Buffer.prototype.readUInt8 = function(offset) {
  var o = (offset | 0);
  if (o < 0 || o >= this.length) throw new RangeError("Index out of range");
  return this[o];
};
Buffer.prototype.readInt8 = function(offset) {
  var v = this.readUInt8(offset);
  return (v & 0x80) ? v - 0x100 : v;
};
Buffer.prototype.readUInt16BE = function(offset) {
  var o = (offset | 0);
  if (o < 0 || o + 2 > this.length) throw new RangeError("Index out of range");
  return (this[o] << 8) | this[o + 1];
};
Buffer.prototype.readUInt16LE = function(offset) {
  var o = (offset | 0);
  if (o < 0 || o + 2 > this.length) throw new RangeError("Index out of range");
  return this[o] | (this[o + 1] << 8);
};
Buffer.prototype.readInt16BE = function(offset) {
  var v = this.readUInt16BE(offset);
  return (v & 0x8000) ? v - 0x10000 : v;
};
Buffer.prototype.readInt16LE = function(offset) {
  var v = this.readUInt16LE(offset);
  return (v & 0x8000) ? v - 0x10000 : v;
};
Buffer.prototype.readUInt32BE = function(offset) {
  var o = (offset | 0);
  if (o < 0 || o + 4 > this.length) throw new RangeError("Index out of range");
  return (this[o] * 0x1000000) + ((this[o + 1] << 16) | (this[o + 2] << 8) | this[o + 3]);
};
Buffer.prototype.readUInt32LE = function(offset) {
  var o = (offset | 0);
  if (o < 0 || o + 4 > this.length) throw new RangeError("Index out of range");
  return ((this[o] | (this[o + 1] << 8) | (this[o + 2] << 16)) + (this[o + 3] * 0x1000000));
};
Buffer.prototype.readInt32BE = function(offset) {
  var o = (offset | 0);
  if (o < 0 || o + 4 > this.length) throw new RangeError("Index out of range");
  return (this[o] << 24) | (this[o + 1] << 16) | (this[o + 2] << 8) | this[o + 3];
};
Buffer.prototype.readInt32LE = function(offset) {
  var o = (offset | 0);
  if (o < 0 || o + 4 > this.length) throw new RangeError("Index out of range");
  return this[o] | (this[o + 1] << 8) | (this[o + 2] << 16) | (this[o + 3] << 24);
};
Buffer.prototype.readBigUInt64BE = function(offset) {
  var hi = BigInt(this.readUInt32BE(offset));
  var lo = BigInt(this.readUInt32BE((offset | 0) + 4));
  return (hi << 32n) | lo;
};
Buffer.prototype.readBigUInt64LE = function(offset) {
  var lo = BigInt(this.readUInt32LE(offset));
  var hi = BigInt(this.readUInt32LE((offset | 0) + 4));
  return (hi << 32n) | lo;
};
Buffer.prototype.writeUInt8 = function(value, offset) {
  var o = (offset | 0);
  if (o < 0 || o >= this.length) throw new RangeError("Index out of range");
  this[o] = Number(value) & 0xff;
  return o + 1;
};
Buffer.prototype.writeInt8 = Buffer.prototype.writeUInt8;
Buffer.prototype.writeUInt16BE = function(value, offset) {
  var o = (offset | 0), v = Number(value) & 0xffff;
  if (o < 0 || o + 2 > this.length) throw new RangeError("Index out of range");
  this[o] = (v >>> 8) & 0xff;
  this[o + 1] = v & 0xff;
  return o + 2;
};
Buffer.prototype.writeUInt16LE = function(value, offset) {
  var o = (offset | 0), v = Number(value) & 0xffff;
  if (o < 0 || o + 2 > this.length) throw new RangeError("Index out of range");
  this[o] = v & 0xff;
  this[o + 1] = (v >>> 8) & 0xff;
  return o + 2;
};
Buffer.prototype.writeInt16BE = Buffer.prototype.writeUInt16BE;
Buffer.prototype.writeInt16LE = Buffer.prototype.writeUInt16LE;
Buffer.prototype.writeUInt32BE = function(value, offset) {
  var o = (offset | 0), v = Number(value) >>> 0;
  if (o < 0 || o + 4 > this.length) throw new RangeError("Index out of range");
  this[o] = (v >>> 24) & 0xff;
  this[o + 1] = (v >>> 16) & 0xff;
  this[o + 2] = (v >>> 8) & 0xff;
  this[o + 3] = v & 0xff;
  return o + 4;
};
Buffer.prototype.writeUInt32LE = function(value, offset) {
  var o = (offset | 0), v = Number(value) >>> 0;
  if (o < 0 || o + 4 > this.length) throw new RangeError("Index out of range");
  this[o] = v & 0xff;
  this[o + 1] = (v >>> 8) & 0xff;
  this[o + 2] = (v >>> 16) & 0xff;
  this[o + 3] = (v >>> 24) & 0xff;
  return o + 4;
};
Buffer.prototype.writeInt32BE = Buffer.prototype.writeUInt32BE;
Buffer.prototype.writeInt32LE = Buffer.prototype.writeUInt32LE;
Buffer.prototype.writeBigUInt64BE = function(value, offset) {
  var o = (offset | 0), v = BigInt(value);
  this.writeUInt32BE(Number((v >> 32n) & 0xffffffffn), o);
  this.writeUInt32BE(Number(v & 0xffffffffn), o + 4);
  return o + 8;
};
Buffer.prototype.writeBigUInt64LE = function(value, offset) {
  var o = (offset | 0), v = BigInt(value);
  this.writeUInt32LE(Number(v & 0xffffffffn), o);
  this.writeUInt32LE(Number((v >> 32n) & 0xffffffffn), o + 4);
  return o + 8;
};
globalThis.Buffer = Buffer;

var __cDepth = 0;
var __cTimers = Object.create(null);
var __cCounts = Object.create(null);

function __cInsp(val, depth, maxDepth, seen) {
  if (val === null) return "null";
  if (val === undefined) return "undefined";
  var t = typeof val;
  if (t === "string") return depth === 0 ? val : "'" + val + "'";
  if (t !== "object") return String(val);
  if (val instanceof Error) return val.name + ": " + val.message;
  if (val instanceof Date) return val.toISOString();
  if (val instanceof RegExp) return val.toString();
  if (Buffer.isBuffer(val)) return "<Buffer " + val.toString("hex") + ">";
  if (val instanceof __U8) return "Uint8Array(" + val.length + ") [" + Array.from(val).join(", ") + "]";
  seen = seen || [];
  if (seen.indexOf(val) !== -1) return "[Circular]";
  if (maxDepth !== null && maxDepth !== undefined && depth > maxDepth) {
    return Array.isArray(val) ? "[Array]" : "[Object]";
  }
  seen.push(val);
  if (Array.isArray(val)) {
    var a = [];
    for (var i = 0; i < val.length; i++) a.push(__cInsp(val[i], depth + 1, maxDepth, seen));
    seen.pop();
    return "[" + a.join(", ") + "]";
  }
  var ks = Object.keys(val), p = [];
  for (var j = 0; j < ks.length; j++) {
    var k = ks[j];
    p.push(k + ": " + __cInsp(val[k], depth + 1, maxDepth, seen));
  }
  seen.pop();
  return p.length === 0 ? "{}" : "{ " + p.join(", ") + " }";
}

function __cPad(str, width) {
  var s = String(str);
  while (s.length < width) s += " ";
  return s;
}

function __cEmitArgs(args) {
  if (__cDepth === 0) {
    if (args.length === 1 && (typeof args[0] !== "object" || args[0] === null)) {
      __slice_console_log(args[0]);
      return;
    }
    var parts0 = [];
    for (var i0 = 0; i0 < args.length; i0++) {
      var a0 = args[i0];
      parts0.push((typeof a0 === "object" && a0 !== null) ? __cInsp(a0, 0, 4, []) : String(a0));
    }
    __slice_console_log(parts0.join(" "));
    return;
  }
  var indent = "";
  for (var d = 0; d < __cDepth; d++) indent += "  ";
  var parts = [];
  for (var i = 0; i < args.length; i++) {
    var a = args[i];
    parts.push((typeof a === "object" && a !== null) ? __cInsp(a, 0, 4, []) : String(a));
  }
  __slice_console_log(indent + parts.join(" "));
}

function __cLog() {
  __cEmitArgs(arguments);
}

var __consoleObj = {
  log: __cLog,
  info: __cLog,
  warn: __cLog,
  error: __cLog,
  debug: __cLog,
  dirxml: __cLog,
  dir: function(obj, options) {
    var maxD = (options && options.depth !== undefined) ? options.depth : 2;
    __cEmitArgs([__cInsp(obj, 1, maxD, [])]);
  },
  trace: function() {
    var parts = ["Trace:"];
    for (var i = 0; i < arguments.length; i++) parts.push(__cInsp(arguments[i], 0, 2, []));
    __cEmitArgs([parts.join(" ")]);
  },
  assert: function(condition) {
    if (condition) return;
    var rest = [];
    for (var i = 1; i < arguments.length; i++) rest.push(__cInsp(arguments[i], 0, 2, []));
    var msg = rest.length > 0 ? "Assertion failed: " + rest.join(" ") : "Assertion failed";
    __cEmitArgs([msg]);
  },
  clear: function() {
    __cDepth = 0;
  },
  count: function(label) {
    var k = label === undefined ? "default" : String(label);
    var n = (__cCounts[k] || 0) + 1;
    __cCounts[k] = n;
    __cEmitArgs([k + ": " + n]);
  },
  countReset: function(label) {
    var k = label === undefined ? "default" : String(label);
    delete __cCounts[k];
  },
  group: function() {
    if (arguments.length > 0) __cEmitArgs(arguments);
    __cDepth++;
  },
  groupCollapsed: function() {
    if (arguments.length > 0) __cEmitArgs(arguments);
    __cDepth++;
  },
  groupEnd: function() {
    if (__cDepth > 0) __cDepth--;
  },
  time: function(label) {
    var k = label === undefined ? "default" : String(label);
    __cTimers[k] = performance.now();
  },
  timeLog: function(label) {
    var k = label === undefined ? "default" : String(label);
    var t0 = __cTimers[k];
    if (t0 === undefined) {
      __cEmitArgs(["Warning: No such label '" + k + "' for console.timeLog()"]);
      return;
    }
    var elapsed = Math.max(0, performance.now() - t0);
    var rest = [k + ": " + elapsed.toFixed(3) + "ms"];
    for (var i = 1; i < arguments.length; i++) rest.push(__cInsp(arguments[i], 0, 2, []));
    __cEmitArgs([rest.join(" ")]);
  },
  timeEnd: function(label) {
    var k = label === undefined ? "default" : String(label);
    var t0 = __cTimers[k];
    if (t0 === undefined) {
      __cEmitArgs(["Warning: No such label '" + k + "' for console.timeEnd()"]);
      return;
    }
    var elapsed = Math.max(0, performance.now() - t0);
    delete __cTimers[k];
    __cEmitArgs([k + ": " + elapsed.toFixed(3) + "ms"]);
  },
  table: function(tabularData, properties) {
    if (tabularData === null || typeof tabularData !== "object") {
      __cEmitArgs([tabularData]);
      return;
    }
    var isArr = Array.isArray(tabularData);
    var rowKeys = isArr ? [] : Object.keys(tabularData);
    if (isArr) {
      for (var r = 0; r < tabularData.length; r++) rowKeys.push(String(r));
    }
    if (rowKeys.length === 0) {
      __cEmitArgs(["(empty table)"]);
      return;
    }
    var colSet = Object.create(null);
    var cols = [];
    var hasValuesCol = false;
    var propFilter = Array.isArray(properties) ? properties.map(String) : null;
    for (var i = 0; i < rowKeys.length; i++) {
      var row = tabularData[rowKeys[i]];
      if (row !== null && typeof row === "object" && !(row instanceof Date) && !(row instanceof RegExp)) {
        var rks = Object.keys(row);
        for (var j = 0; j < rks.length; j++) {
          var ck = rks[j];
          if (propFilter && propFilter.indexOf(ck) === -1) continue;
          if (!colSet[ck]) { colSet[ck] = true; cols.push(ck); }
        }
      } else {
        hasValuesCol = true;
      }
    }
    if (hasValuesCol) cols.push("Values");
    var headers = ["(index)"].concat(cols);
    var widths = [];
    for (var h = 0; h < headers.length; h++) widths.push(headers[h].length);
    var matrix = [];
    for (var ri = 0; ri < rowKeys.length; ri++) {
      var rk = rowKeys[ri];
      var rv = tabularData[rk];
      var cells = [rk];
      if (rk.length > widths[0]) widths[0] = rk.length;
      var isObj = rv !== null && typeof rv === "object" && !(rv instanceof Date) && !(rv instanceof RegExp);
      for (var ci = 0; ci < cols.length; ci++) {
        var colName = cols[ci];
        var cellStr = "";
        if (colName === "Values" && !isObj) {
          cellStr = __cInsp(rv, 1, 1, []);
        } else if (isObj && Object.prototype.hasOwnProperty.call(rv, colName)) {
          cellStr = __cInsp(rv[colName], 1, 1, []);
        }
        cells.push(cellStr);
        if (cellStr.length > widths[ci + 1]) widths[ci + 1] = cellStr.length;
      }
      matrix.push(cells);
    }
    var sepParts = [];
    var hdrParts = [];
    for (var wi = 0; wi < headers.length; wi++) {
      var w = widths[wi];
      var dashes = "";
      for (var di = 0; di < w + 2; di++) dashes += "-";
      sepParts.push(dashes);
      hdrParts.push(" " + __cPad(headers[wi], w) + " ");
    }
    var border = "+" + sepParts.join("+") + "+";
    var lines = [border, "|" + hdrParts.join("|") + "|", border];
    for (var mi = 0; mi < matrix.length; mi++) {
      var rowCells = matrix[mi], rParts = [];
      for (var cj = 0; cj < rowCells.length; cj++) {
        rParts.push(" " + __cPad(rowCells[cj], widths[cj]) + " ");
      }
      lines.push("|" + rParts.join("|") + "|");
    }
    lines.push(border);
    __cEmitArgs([lines.join("\n")]);
  }
};
globalThis.console = __consoleObj;
})();"##;

