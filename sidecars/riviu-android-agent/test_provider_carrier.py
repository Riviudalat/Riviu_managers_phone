"""Host owner tests for the real probe/provider against a minimal Android API fixture.

These protect transport selection, handle release and endpoint admission; they do
not qualify Android hidden APIs, APK installation, SELinux or phone ownership.
"""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess

ROOT = Path(__file__).resolve().parent
OUT = ROOT / "build/probe/binder-admission-20261003/owner-tests"
FIXTURES = {
    "android/os/IBinder.java": "package android.os; public interface IBinder { boolean transact(int c, Parcel d, Parcel r, int f); }",
    "android/os/Binder.java": "package android.os; public class Binder implements IBinder { public static int uid=2000; public static int getCallingUid(){return uid;} public boolean transact(int c,Parcel d,Parcel r,int f){if(c!=1)throw new AssertionError(\"readonly transaction\");r.writeInt(10180);for(int i=1;i<=4;i++)r.writeString((String)d.values.get(i));return true;} }",
    "android/os/Parcel.java": "package android.os; public class Parcel { public java.util.List<Object> values=new java.util.ArrayList<>(); int pos; public static Parcel obtain(){return new Parcel();} public void writeInterfaceToken(String s){values.add(s);} public void writeString(String s){values.add(s);} public void writeInt(int i){values.add(i);} public String readString(){return (String)values.get(pos++);} public int readInt(){return (Integer)values.get(pos++);} public void readException(){} public int dataAvail(){return values.size()-pos;} public void recycle(){} }",
    "android/os/Bundle.java": "package android.os; public class Bundle { public java.util.Map<String,IBinder> v=new java.util.HashMap<>(); public void putBinder(String k,IBinder b){v.put(k,b);} public IBinder getBinder(String k){return v.get(k);} public int size(){return v.size();} }",
    "android/os/Process.java": "package android.os; public class Process { public static int myUid(){return 2000;} }",
    "android/os/Build.java": "package android.os; public class Build { public static class VERSION { public static int SDK_INT=28; } }",
    "android/os/Looper.java": "package android.os; public class Looper { public static Looper myLooper(){return new Looper();} public static void prepareMainLooper(){} public static void loop(){} }",
    "android/content/ComponentName.java": "package android.content; public class ComponentName { public final String p,n; public ComponentName(String p,String n){this.p=p;this.n=n;} public boolean equals(Object o){return o instanceof ComponentName && p.equals(((ComponentName)o).p)&&n.equals(((ComponentName)o).n);} }",
    "android/content/Intent.java": "package android.content; public class Intent { public Intent(String a){} public Intent setComponent(ComponentName c){return this;} }",
    "android/content/ServiceConnection.java": "package android.content; public interface ServiceConnection { void onServiceConnected(ComponentName n,android.os.IBinder b); void onServiceDisconnected(ComponentName n); default void onNullBinding(ComponentName n){} default void onBindingDied(ComponentName n){} }",
    "android/content/Context.java": "package android.content; public class Context { public static final int BIND_AUTO_CREATE=1; public static int dump=0; public android.content.pm.PackageManager getPackageManager(){return new android.content.pm.PackageManager();} public int checkSelfPermission(String p){return dump;} public int checkCallingPermission(String p){return dump;} public Context createPackageContext(String p,int f){return this;} public boolean bindService(Intent i,ServiceConnection c,int f){throw new SecurityException(\"Unable to find app for caller fixture when binding service fixture\");} public void unbindService(ServiceConnection c){throw new AssertionError(\"not a bound service\");} }",
    "android/content/pm/ApplicationInfo.java": "package android.content.pm; public class ApplicationInfo { public int uid=10180; public boolean enabled=true; }",
    "android/content/pm/ServiceInfo.java": "package android.content.pm; public class ServiceInfo { public ApplicationInfo applicationInfo=new ApplicationInfo(); public boolean enabled=true,exported=true; public String packageName=\"com.riviu.agent\",name=\"com.riviu.agent.AgentService\",permission=\"android.permission.DUMP\"; }",
    "android/content/pm/ProviderInfo.java": "package android.content.pm; public class ProviderInfo { public ApplicationInfo applicationInfo=new ApplicationInfo(); public boolean enabled=true,exported=true; public String packageName=\"com.riviu.agent\",name=\"com.riviu.agent.BootstrapBinderProvider\",authority=\"com.riviu.agent.bootstrap\",readPermission=\"android.permission.DUMP\",writePermission=\"android.permission.DUMP\"; }",
    "android/content/pm/Signature.java": "package android.content.pm; public class Signature { public byte[] toByteArray(){return new byte[]{1,2,3};} }",
    "android/content/pm/PackageInfo.java": "package android.content.pm; public class PackageInfo { public String packageName=\"com.riviu.agent\",versionName=\"0.7.0\"; public int versionCode=7; public ApplicationInfo applicationInfo=new ApplicationInfo(); public Signature[] signatures={new Signature()}; }",
    "android/content/pm/PackageManager.java": "package android.content.pm; public class PackageManager { public static final int GET_SIGNATURES=64,PERMISSION_GRANTED=0; public static boolean tamper; public ServiceInfo getServiceInfo(android.content.ComponentName n,int f){return new ServiceInfo();} public PackageInfo getPackageInfo(String p,int f){return new PackageInfo();} public ProviderInfo getProviderInfo(android.content.ComponentName n,int f){ProviderInfo p=new ProviderInfo(); if(tamper)p.exported=false;return p;} }",
    "android/content/IContentProvider.java": "package android.content; public interface IContentProvider { android.os.Bundle call(String pkg,String method,String arg,android.os.Bundle extras); }",
    "android/app/IActivityManager.java": "package android.app; public interface IActivityManager { ContentProviderHolder getContentProviderExternal(String name,int user,android.os.IBinder token); void removeContentProviderExternal(String name,android.os.IBinder token); }",
    "android/app/ContentProviderHolder.java": "package android.app; public class ContentProviderHolder { public android.content.IContentProvider provider; public ContentProviderHolder(android.content.IContentProvider p){provider=p;} }",
    "android/app/ActivityManager.java": "package android.app; public class ActivityManager { public static int acquired,released; public static boolean badReply; public static IActivityManager getService(){return new IActivityManager(){ public ContentProviderHolder getContentProviderExternal(String n,int u,android.os.IBinder t){if(!n.equals(\"com.riviu.agent.bootstrap\")||u!=0||t==null)throw new AssertionError(\"external acquisition identity\");acquired++; return new ContentProviderHolder((pkg,method,arg,extras)->{if(!pkg.equals(\"com.android.shell\")||!method.equals(\"binder\")||arg!=null||extras!=null)throw new AssertionError(\"credential-free acquisition\");android.os.Bundle b=new android.os.Bundle(); if(!badReply)b.putBinder(\"binder\",new android.os.Binder());return b;});} public void removeContentProviderExternal(String n,android.os.IBinder t){released++; System.err.println(\"fixture_external_release=\"+released);} };} }",
    "android/content/ContentProvider.java": "package android.content; public abstract class ContentProvider { public Context getContext(){return new Context();} public abstract boolean onCreate(); public android.os.Bundle call(String m,String a,android.os.Bundle e){return null;} public abstract android.database.Cursor query(android.net.Uri u,String[] p,String s,String[] a,String o); public abstract String getType(android.net.Uri u); public abstract android.net.Uri insert(android.net.Uri u,ContentValues v); public abstract int delete(android.net.Uri u,String s,String[] a); public abstract int update(android.net.Uri u,ContentValues v,String s,String[] a); }",
    "android/content/ContentValues.java": "package android.content; public class ContentValues {}",
    "android/database/Cursor.java": "package android.database; public interface Cursor {}",
    "android/net/Uri.java": "package android.net; public class Uri {}",
    "org/json/JSONObject.java": "package org.json; public class JSONObject { public String getString(String k){return \"0.7.0\";} public int getInt(String k){return 1;} public JSONArray getJSONArray(String k){return new JSONArray();} }",
    "org/json/JSONArray.java": "package org.json; public class JSONArray { public int length(){return 1;} public String getString(int i){return \"secureBootstrapBinderShell\";} }",
    "com/riviu/agent/AgentService.java": "package com.riviu.agent; public class AgentService { public static android.os.IBinder active=new android.os.Binder(); static android.os.IBinder activeBootstrapBinder(){return active;} }",
    "com/riviu/agent/ProbeOwnerCheck.java": """package com.riviu.agent;
import java.lang.reflect.*;
public class ProbeOwnerCheck {
 public static void main(String[] args)throws Exception {
  Field d=ProbeBinder.class.getDeclaredField("deadline");d.setAccessible(true);d.set(null,System.nanoTime()+5000000000L);
  Method x=ProbeBinder.class.getDeclaredMethod("exchange",android.content.Context.class,int.class,String.class,String.class,String.class,String.class,String.class);x.setAccessible(true);
  String cert="039058c6f2c0cb492c533b0a4d14ef77cc0f78abccced5287d84a1a2011cfb81";
  x.invoke(null,new android.content.Context(),10180,cert,"nonce_fixture_12345","owner_fixture_12345","-","-");
 }
}""",
    "com/riviu/agent/ProviderOwnerCheck.java": """package com.riviu.agent;
public class ProviderOwnerCheck {
 static void check(boolean c){if(!c)throw new AssertionError("provider admission/handle contract");}
 public static void main(String[] args)throws Exception {
  BootstrapBinderProvider p=new BootstrapBinderProvider(); check(p.onCreate());
  for(int uid:new int[]{0,10000,2000}) {
   android.os.Binder.uid=uid;android.content.Context.dump=(uid==2000?-1:0);
   try {p.call("binder",null,null);throw new AssertionError("unauthorized acquisition");}catch(SecurityException expected){}
   try {p.call("unrecognized","private_fixture",new android.os.Bundle());throw new AssertionError("unauthorized metadata");}catch(SecurityException expected){}
  }
  android.os.Binder.uid=2000;android.content.Context.dump=0;
  check(p.call("binder",null,null).getBinder("binder")==AgentService.active);
  AgentService.active=null;
  try {p.call("binder",null,null);throw new AssertionError("stopped service");}catch(IllegalStateException expected){}
  AgentService.active=new android.os.Binder();
  try {p.call("binder","unexpected",null);throw new AssertionError("metadata accepted");}catch(IllegalArgumentException expected){}
  String cert="039058c6f2c0cb492c533b0a4d14ef77cc0f78abccced5287d84a1a2011cfb81";
  android.app.ActivityManager.badReply=true;
  try {BootstrapProviderCarrier.acquire(new android.content.Context(),10180,cert);throw new AssertionError("bad provider reply");}catch(java.io.IOException expected){}
  check(android.app.ActivityManager.acquired==1&&android.app.ActivityManager.released==1);
  android.content.pm.PackageManager.tamper=true;
  try {BootstrapProviderCarrier.acquire(new android.content.Context(),10180,cert);throw new AssertionError("unpinned provider");}catch(java.io.IOException expected){}
  check(android.app.ActivityManager.acquired==1);
  android.content.pm.PackageManager.tamper=false;android.app.ActivityManager.badReply=false;
  try(BootstrapProviderCarrier h=BootstrapProviderCarrier.acquire(new android.content.Context(),10180,cert)){check(h.binder()!=null);}
  check(android.app.ActivityManager.acquired==2&&android.app.ActivityManager.released==2);
  System.out.println("PASS kernel UID+DUMP, stopped service, metadata refusal, provider pins and external handle release");
 }
}""",
}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--probe-only", action="store_true")
    parser.add_argument("--source-root", type=Path, default=ROOT,
                        help="Isolated source copy for baseline/rollback verification")
    args = parser.parse_args()
    fixtures = OUT / "fixtures"
    classes = OUT / "classes"
    classes.mkdir(parents=True, exist_ok=True)
    source_files = []
    for name, text in FIXTURES.items():
        if args.probe_only and name.endswith("ProviderOwnerCheck.java"):
            continue
        path = fixtures / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")
        source_files.append(path)
    native = args.source_root / "app/src/main/java/com/riviu/agent"
    sources = [args.source_root / "ProbeBinder.java"]
    for name in ("BootstrapProviderCarrier.java", "BootstrapBinderProvider.java"):
        if (native / name).exists():
            sources.append(native / name)
    javac = Path(shutil.which("javac"))
    java = javac.with_name("java.exe" if os.name == "nt" else "java")
    ledger = []

    def run(argv):
        result = subprocess.run([str(x) for x in argv], capture_output=True, text=True)
        ledger.append(dict(command=[str(x) for x in argv], stdout=result.stdout,
                           stderr=result.stderr, exit=result.returncode))
        (OUT / "VERIFICATION.json").write_text(json.dumps(ledger, indent=2), encoding="utf-8")
        print(result.stdout, end="")
        print(result.stderr, end="")
        return result

    compiled = run([javac, "-source", "8", "-target", "8", "-nowarn", "-d", classes,
                    *source_files, *sources])
    if compiled.returncode:
        raise SystemExit(compiled.returncode)
    probe = run([java, "-cp", classes, "com.riviu.agent.ProbeOwnerCheck"])
    if probe.returncode != 0 or '"ok":true' not in probe.stdout or "fixture_external_release=1" not in probe.stderr:
        raise SystemExit("FAIL readonly probe must use admitted external provider and release its handle")
    if not args.probe_only:
        provider = run([java, "-cp", classes, "com.riviu.agent.ProviderOwnerCheck"])
        if provider.returncode:
            raise SystemExit(provider.returncode)
    print("PASS readonly external-provider probe owner contract")


if __name__ == "__main__":
    main()
