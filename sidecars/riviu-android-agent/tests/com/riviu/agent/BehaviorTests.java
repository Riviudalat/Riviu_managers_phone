package com.riviu.agent;

import java.io.ByteArrayInputStream;
import java.io.ByteArrayOutputStream;
import java.io.Closeable;
import java.io.DataOutputStream;
import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicInteger;

/** Runs production state/scheduler/ownership seams, not source-string assertions. */
public final class BehaviorTests {
    private static int assertions;
    private static void check(boolean value, String message) {
        assertions++;
        if (!value) throw new AssertionError(message);
    }
    private static final class Scheduler implements MainThreadJobs.Scheduler, MainThreadJobs.Clock {
        final List<Runnable> posted = new ArrayList<Runnable>();
        long now;
        boolean accept = true;
        @Override public boolean post(Runnable runnable) { posted.add(runnable); return accept; }
        @Override public void remove(Runnable runnable) { /* deliberately leaves stale runnable */ }
        @Override public long now() { return now; }
    }
    private static final class Resource implements Closeable {
        volatile boolean closed;
        @Override public void close() { closed = true; }
    }
    private static void clipboard() throws Exception {
        Scheduler scheduler = new Scheduler();
        MainThreadJobs jobs = new MainThreadJobs(scheduler, scheduler, 3);
        AtomicInteger writes = new AtomicInteger();
        MainThreadJobs.Job<Integer> queued = jobs.submit(writes::incrementAndGet, 5, TimeUnit.SECONDS);
        check(queued.state() == MainThreadJobs.State.QUEUED, "initial queued");
        try { queued.await(0, TimeUnit.SECONDS); throw new AssertionError("timeout must fail"); }
        catch (MainThreadJobs.Pending e) { check(e.state == MainThreadJobs.State.CANCELLED, "queued timeout cancels"); }
        scheduler.posted.get(0).run();
        check(writes.get() == 0, "stale dispatch cannot write after timeout");
        check(!queued.cancelQueued(), "cancel idempotent");
        MainThreadJobs.Job<Integer> expired = jobs.submit(writes::incrementAndGet, 1, TimeUnit.NANOSECONDS);
        scheduler.now = 2;
        expired.run();
        check(expired.state() == MainThreadJobs.State.CANCELLED && writes.get() == 0, "deadline checked at dispatch");
        scheduler.now = 0;
        CountDownLatch running = new CountDownLatch(1), finish = new CountDownLatch(1);
        MainThreadJobs.Job<Integer> active = jobs.submit(() -> {
            running.countDown();
            try { finish.await(); } catch (InterruptedException e) { throw new IllegalStateException(e); }
            return writes.incrementAndGet();
        }, 5, TimeUnit.SECONDS);
        Thread main = new Thread(active);
        main.start();
        check(running.await(2, TimeUnit.SECONDS), "running dispatch reached");
        check(!active.cancelQueued(), "running cannot be cancelled");
        try { active.await(0, TimeUnit.SECONDS); throw new AssertionError("running timeout"); }
        catch (MainThreadJobs.Pending e) { check(e.state == MainThreadJobs.State.RUNNING, "running timeout remains uncertain"); }
        jobs.closeAdmission();
        check(active.state() == MainThreadJobs.State.RUNNING && jobs.unsettled() == 1, "close does not fake running settlement");
        finish.countDown(); main.join(2000);
        check(active.await(1, TimeUnit.SECONDS) == 1, "running effect settles once");
        check(jobs.unsettled() == 0, "all settled");
        try { jobs.submit(writes::incrementAndGet, 1, TimeUnit.SECONDS); throw new AssertionError("closed admission"); }
        catch (IllegalStateException expected) { check(true, "closed rejects"); }

        Scheduler names = new Scheduler();
        MainThreadJobs ledger = new MainThreadJobs(names, names, 1);
        MainThreadJobs.Job<Integer> named = ledger.submit("request_123456789", "set:hash", writes::incrementAndGet, 5, TimeUnit.SECONDS);
        named.run();
        check(ledger.submit("request_123456789", "set:hash", writes::incrementAndGet, 5, TimeUnit.SECONDS) == named,
                "same request gets same settlement without redispatch");
        check(writes.get() == 2 && names.posted.size() == 1, "dedupe no late replay");
        try { ledger.submit("request_123456789", "different", writes::incrementAndGet, 5, TimeUnit.SECONDS); throw new AssertionError("conflict"); }
        catch (IllegalArgumentException expected) { check(true, "conflicting request denied"); }
        try { ledger.submit("request_987654321", "new", writes::incrementAndGet, 5, TimeUnit.SECONDS); throw new AssertionError("ledger full"); }
        catch (IllegalStateException expected) { check(ledger.find(named.id) == named, "named settlement never evicted"); }
        Scheduler reject = new Scheduler(); reject.accept = false;
        MainThreadJobs.Job<Integer> rejected = new MainThreadJobs(reject, reject, 1).submit(writes::incrementAndGet, 5, TimeUnit.SECONDS);
        check(rejected.state() == MainThreadJobs.State.FAILED, "post failure terminal");
        rejected.run(); check(writes.get() == 2, "failed post never dispatches");
        Scheduler errors = new Scheduler();
        MainThreadJobs.Job<Integer> failed = new MainThreadJobs(errors, errors, 1).submit(() -> { throw new IllegalStateException("test"); }, 5, TimeUnit.SECONDS);
        failed.run(); check(failed.state() == MainThreadJobs.State.FAILED, "action failure retained");
        check(!ClipboardStore.Value.read(null).available, "null clip is unavailable");
        check(ClipboardStore.Value.read("").available, "readable empty clip is valid");
    }
    private static void service() {
        ServiceStartPolicy policy = new ServiceStartPolicy();
        check(policy.decide(null, null, false, false, false) == ServiceStartPolicy.Decision.WAIT, "cold sticky waits");
        check(policy.decide(null, "a", true, true, false) == ServiceStartPolicy.Decision.WAIT, "warm tokenless no rekey");
        check(policy.decide("a", null, false, false, false) == ServiceStartPolicy.Decision.START, "initial start");
        check(policy.decide("a", "a", true, true, false) == ServiceStartPolicy.Decision.KEEP, "same token live no-op");
        check(policy.decide("b", "a", true, true, false) == ServiceStartPolicy.Decision.REFUSE_OWNER, "different token not owner proof");
        check(policy.decide("b", "a", true, false, false) == ServiceStartPolicy.Decision.REFUSE_OWNER, "dead listener not takeover proof");
        check(policy.decide("a", "a", true, false, true) == ServiceStartPolicy.Decision.REFUSE_BUSY, "dead listener busy must settle");
        for (int i = 0; i < 3; i++) check(policy.decide("a", "a", true, false, false) == ServiceStartPolicy.Decision.RECOVER, "same-token dead listener recovery");
        check(policy.decide("a", "a", true, false, false) == ServiceStartPolicy.Decision.REFUSE_RECOVERY, "recovery bounded");
    }
    private static void resources() throws Exception {
        OwnedRequests pool = new OwnedRequests(1, 1);
        CountDownLatch running = new CountDownLatch(1), finish = new CountDownLatch(1);
        Resource first = new Resource(), queued = new Resource(), excess = new Resource();
        AtomicInteger effects = new AtomicInteger();
        check(pool.accept(first, () -> {
            running.countDown();
            try { finish.await(); } catch (InterruptedException e) { throw new AssertionError("running effect interrupted", e); }
            effects.incrementAndGet();
        }), "first accepted");
        check(running.await(2, TimeUnit.SECONDS), "worker entered");
        check(pool.accept(queued, effects::incrementAndGet), "bounded queue slot");
        check(!pool.accept(excess, effects::incrementAndGet) && excess.closed, "overflow socket closed");
        check(pool.ownedCount() == 2, "only bounded sockets owned");
        long start = System.nanoTime(); pool.closeAdmission();
        check(System.nanoTime() - start < TimeUnit.SECONDS.toNanos(1), "stop does not await on caller/main");
        check(first.closed && queued.closed && effects.get() == 0, "all sockets closed, queued effect never runs");
        check(!pool.awaitDrain(1, TimeUnit.MILLISECONDS), "running remains unsettled");
        finish.countDown();
        check(pool.awaitDrain(2, TimeUnit.SECONDS) && effects.get() == 1, "running drains without interrupt");
        check(pool.ownedCount() == 0, "no socket leak after drain");
        Resource later = new Resource();
        check(!pool.accept(later, effects::incrementAndGet) && later.closed, "closing reject closes resource");
        pool.closeAdmission();
    }
    private static void bootstrap() throws Exception {
        byte[] canary = "canary_private_payload".getBytes("UTF-8");
        ByteArrayOutputStream stream = new ByteArrayOutputStream();
        BootstrapEnvelope.write(stream, canary);
        check(java.util.Arrays.equals(canary, BootstrapEnvelope.read(new ByteArrayInputStream(stream.toByteArray()))), "bounded frame roundtrip");
        for (int length : new int[] { -1, 0, 4097, Integer.MAX_VALUE }) {
            stream.reset(); new DataOutputStream(stream).writeInt(length);
            try { BootstrapEnvelope.read(new ByteArrayInputStream(stream.toByteArray())); throw new AssertionError("oversize"); }
            catch (java.io.IOException expected) { check(!expected.getMessage().contains("canary"), "bad size redacted"); }
        }
        check(BootstrapEnvelope.serverUidAllowed(10123, 10123), "checked package UID exact");
        check(!BootstrapEnvelope.serverUidAllowed(10124, 10123), "socket collision fails UID");
        check(!BootstrapEnvelope.serverUidAllowed(2000, 2000), "shell not package UID");
        check(BootstrapEnvelope.callerUidAllowed(10123, 10123, true), "debug same-package run-as caller allowed");
        check(!BootstrapEnvelope.callerUidAllowed(2000, 10123, true), "shell caller denied");
        check(!BootstrapEnvelope.callerUidAllowed(0, 10123, true)
                && !BootstrapEnvelope.callerUidAllowed(10124, 10123, true), "root/other package denied");
        check(!BootstrapEnvelope.callerUidAllowed(10123, 10123, false), "release package caller blocked");
        check(!BootstrapEnvelope.callerUidAllowed(110123, 110123, true), "non-user0 run-as caller blocked");
    }
    private static void owner() {
        OwnerSession session = new OwnerSession();
        check(!OwnerSession.bindingMatches("nonceA", "ownerA", "nonceB", "ownerA"), "wrong nonce refused");
        check(!OwnerSession.bindingMatches("nonceA", "ownerA", "nonceA", "ownerB"), "wrong bootstrap owner refused");
        check(OwnerSession.bindingMatches("nonceA", "ownerA", "nonceA", "ownerA"), "exact bootstrap binding");
        check(session.claim("ownerA", "secretA", true) == OwnerSession.Claim.CONFLICT, "legacy occupied blocks claim");
        check(session.claim("ownerA", "secretA", false) == OwnerSession.Claim.NEW, "fresh claim");
        String instance = session.instance(), generation = session.generation();
        check(instance != null && generation != null && !instance.equals(generation), "independent instance generation");
        check(session.claim("ownerA", "secretA", false) == OwnerSession.Claim.RESUME, "same owner token resumes");
        check(instance.equals(session.instance()) && generation.equals(session.generation()), "resume stable proof");
        check(session.claim("ownerB", "secretA", false) == OwnerSession.Claim.CONFLICT, "token alone no takeover");
        check(session.claim("ownerA", "secretB", false) == OwnerSession.Claim.CONFLICT, "owner alone no rekey");
        check(!session.release("ownerA", "secretA", "stale"), "stale release refused");
        check(!session.release("ownerB", "secretA", instance), "other owner release refused");
        check(session.release("ownerA", "secretA", instance), "exact release admission closes");
        check(!session.finishRelease(false) && session.owner().equals("ownerA"), "unsettled release preserves ownership");
        check(session.claim("ownerB", "secretB", false) == OwnerSession.Claim.CONFLICT, "draining cannot takeover");
        check(session.finishRelease(true) && session.owner() == null, "settled release clears ownership");
        check(session.claim("ownerB", "secretB", false) == OwnerSession.Claim.NEW, "fresh owner after settled release");
        check(!instance.equals(session.instance()) && !generation.equals(session.generation()), "new owner new proof");
        check(!session.release("ownerA", "secretA", instance), "old release cannot kill new owner");
        for (int i = 0; i < 3; i++) check(session.recover(), "secure listener bounded recovery available");
        check(!session.recover(), "secure listener recovery bounded");
        check(!BootstrapEnvelope.serverUidAllowed(110123, 110123), "non-user0 UID excluded");
    }
    private static void binderBootstrap() throws Exception {
        Class<?> carrier;
        try { carrier = Class.forName("com.riviu.agent.BootstrapBinderPolicy"); }
        catch (ClassNotFoundException absent) { throw new AssertionError("release Binder bootstrap carrier missing"); }
        java.lang.reflect.Method caller = carrier.getDeclaredMethod("callerAllowed", int.class, boolean.class);
        for (int uid : new int[] { 0, 2000, 1000, 10123, 102000 }) {
            check((Boolean) caller.invoke(null, uid, true) == (uid == 2000), "only kernel shell UID can bootstrap");
        }
        check(!(Boolean) caller.invoke(null, 2000, false), "shell without DUMP refused");
        java.lang.reflect.Method identity = carrier.getDeclaredMethod("identityAllowed", String.class,
                String.class, String.class, int.class, int.class, String.class, String.class, boolean.class, boolean.class);
        String pin = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        Object[] exact = { "com.riviu.agent", "com.riviu.agent.AgentService", "android.permission.DUMP",
                10123, 10123, pin, pin, true, true };
        check((Boolean) identity.invoke(null, exact), "exact pinned service identity");
        Object[][] rejected = { { 0, "com.other" }, { 1, "com.riviu.agent.OtherService" },
                { 2, null }, { 3, 0 }, { 3, 10124 }, { 5, "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb" },
                { 7, false }, { 8, false } };
        for (Object[] change : rejected) {
            Object[] altered = exact.clone(); altered[(Integer) change[0]] = change[1];
            check(!(Boolean) identity.invoke(null, altered), "unproven service cannot receive credentials");
        }
        java.lang.reflect.Method binding = carrier.getDeclaredMethod("sessionMatches", String.class,
                String.class, String.class, String.class, String.class);
        check((Boolean) binding.invoke(null, "claim", "-", "-", null, null), "cold claim admitted");
        check((Boolean) binding.invoke(null, "claim", "-", "-", "instance", "generation"), "lost ACK same-owner claim may reconcile");
        check(!(Boolean) binding.invoke(null, "release", "instance", "stale", "instance", "generation"), "stale generation cannot release");
        check(!(Boolean) binding.invoke(null, "release", "-", "-", "instance", "generation"), "unbound release refused");
        check((Boolean) binding.invoke(null, "release", "instance", "generation", "instance", "generation"), "exact release binding");
    }
    private static void acceptDeadline() throws Exception {
        final long[] now = { 0 };
        AtomicInteger polls = new AtomicInteger();
        boolean accepted = AcceptDeadline.await(timeout -> {
            polls.incrementAndGet(); now[0] += TimeUnit.MILLISECONDS.toNanos(timeout); return false;
        }, () -> now[0], () -> false, 4500);
        check(!accepted && polls.get() == 45, "no-client deadline terminates polling worker");
        now[0] = 0;
        check(!AcceptDeadline.await(timeout -> { now[0] = TimeUnit.MILLISECONDS.toNanos(4501); return true; },
                () -> now[0], () -> false, 4500), "late readiness cannot dispatch");
        now[0] = 0;
        final boolean[] closed = { false };
        check(!AcceptDeadline.await(timeout -> { closed[0] = true; return true; },
                () -> now[0], () -> closed[0], 4500), "closed readiness cannot dispatch");
        check(AcceptDeadline.await(timeout -> true, () -> 0, () -> false, 4500), "timely readiness admitted");
        check(!AcceptDeadline.await(timeout -> { throw new AssertionError("must not poll closed listener"); },
                () -> 0, () -> true, 4500), "already-closed admission no poll");
        now[0] = 0; closed[0] = false;
        AtomicInteger dispatches = new AtomicInteger();
        long deadline = TimeUnit.MILLISECONDS.toNanos(4500);
        check(AcceptDeadline.mayDispatch(() -> now[0], () -> closed[0], deadline), "timely main enqueue allowed");
        Runnable delayedMain = () -> {
            if (AcceptDeadline.mayDispatch(() -> now[0], () -> closed[0], deadline)) dispatches.incrementAndGet();
        };
        now[0] = deadline;
        delayedMain.run(); check(dispatches.get() == 0, "delayed main runnable cannot claim after deadline");
        now[0] = 0; closed[0] = true;
        delayedMain.run(); check(dispatches.get() == 0, "closed main runnable cannot claim");
        ByteArrayOutputStream framed = new ByteArrayOutputStream();
        BootstrapEnvelope.write(framed, new byte[] { 1, 2, 3 });
        final ByteArrayInputStream raw = new ByteArrayInputStream(framed.toByteArray());
        final long[] slowClock = { 0 };
        java.io.InputStream trickle = new java.io.InputStream() {
            @Override public int read() { slowClock[0] += TimeUnit.MILLISECONDS.toNanos(1000); return raw.read(); }
            @Override public int read(byte[] target, int offset, int size) {
                int value = read(); if (value < 0) return -1; target[offset] = (byte) value; return 1;
            }
        };
        List<Integer> timeouts = new ArrayList<Integer>();
        try {
            BootstrapEnvelope.readDeadline(trickle, deadline, () -> slowClock[0], timeouts::add);
            throw new AssertionError("trickle must time out absolutely");
        } catch (java.io.IOException expected) {
            check(slowClock[0] == TimeUnit.MILLISECONDS.toNanos(5000), "trickle bounded by absolute clock");
            check(timeouts.get(0) == 4500 && timeouts.get(timeouts.size() - 1) == 500,
                    "remaining socket timeout shrinks per read");
        }
    }
    private static void auth() {
        check(RequestAuth.allowed("GET", "/status", "secret", null), "public identification allowed");
        check(!RequestAuth.allowed("POST", "/v1/session/status", "secret", "wrong"), "public status does not authorize session status");
        check(!RequestAuth.allowed("POST", "/v1/clipboard/set", "secret", null), "mutation missing token denied");
        check(RequestAuth.allowed("POST", "/v1/apps/describe", "secret", "secret"), "describe authenticates separately");
        check(!RequestAuth.allowed("POST", "/v1/session/status", "", ""), "empty config fails closed");
        RequestAuth.requireId("nonce_12345678901"); check(true, "bounded nonce accepted");
        try { RequestAuth.requireId("short"); throw new AssertionError("nonce too short"); }
        catch (IllegalArgumentException e) { check(true, "short nonce rejected"); }
    }
    private static void compareClipboard() {
        final class Clipboard implements ClipboardCompare.Access {
            String text = "sentinel";
            int writes;
            boolean unreadableAfterWrite;
            @Override public String read() { return unreadableAfterWrite && writes != 0 ? null : text; }
            @Override public void write(String replacement) { writes++; text = replacement; }
        }
        Clipboard clipboard = new Clipboard();
        Scheduler scheduler = new Scheduler();
        MainThreadJobs jobs = new MainThreadJobs(scheduler, scheduler, 8);
        String fingerprint = ClipboardStore.fingerprint("compareSet", "sentinel", "baseline");
        MainThreadJobs.Job<ClipboardCompare.Result> queued = jobs.submit("restore_request_123", fingerprint,
                () -> ClipboardCompare.apply(clipboard, "sentinel", "baseline"), 5, TimeUnit.SECONDS);
        clipboard.text = "foreign changed while queued";
        queued.run();
        ClipboardCompare.Result mismatch = queued.result();
        check(!mismatch.written && mismatch.changed && clipboard.writes == 0, "foreign queued change never overwritten");
        check(queued.state() == MainThreadJobs.State.SUCCEEDED, "compare mismatch is known settlement not unknown/error");
        clipboard.text = "sentinel";
        ClipboardCompare.Result restored = ClipboardCompare.apply(clipboard, "sentinel", "baseline");
        check(restored.written && restored.verified && !restored.changed && clipboard.writes == 1, "exact expected restores once with readback");
        clipboard.text = null;
        ClipboardCompare.Result unavailable = ClipboardCompare.apply(clipboard, "sentinel", "baseline");
        check(!unavailable.available && !unavailable.written && clipboard.writes == 1, "unavailable clip never overwritten");
        clipboard.text = "";
        ClipboardCompare.Result empty = ClipboardCompare.apply(clipboard, "", "");
        check(empty.available && empty.written && empty.verified, "readable empty compare remains valid");
        Clipboard lostClipboard = new Clipboard(); lostClipboard.unreadableAfterWrite = true;
        ClipboardCompare.Result lostReadback = ClipboardCompare.apply(lostClipboard, "sentinel", "baseline");
        check(lostReadback.written && !lostReadback.verified && !lostReadback.available
                && lostClipboard.writes == 1, "lost readback retains unverified write no replay");
        check(!fingerprint.equals(ClipboardStore.fingerprint("compareSet", "other expected", "baseline")), "dedupe fingerprint binds expected");
        check(!fingerprint.equals(ClipboardStore.fingerprint("compareSet", "sentinel", "other replacement")), "dedupe fingerprint binds replacement");
        check(!ClipboardStore.fingerprint("compareSet", "a\u0000b", "c").equals(
                ClipboardStore.fingerprint("compareSet", "a", "b\u0000c")), "length framing rejects separator collision");
        check(jobs.<ClipboardCompare.Result>submit("restore_request_123", fingerprint, () -> { throw new AssertionError("must not replay"); },
                5, TimeUnit.SECONDS) == queued, "same request reuses mismatch settlement without write");
        try { jobs.submit("restore_request_123", ClipboardStore.fingerprint("compareSet", "new", "baseline"),
                () -> ClipboardCompare.apply(clipboard, "new", "baseline"), 5, TimeUnit.SECONDS);
            throw new AssertionError("changed expected replay");
        } catch (IllegalArgumentException expected) { check(true, "changed expected same request conflict"); }
    }
    private static void clipboardSnapshots() {
        final class Clip {
            final String text, label;
            final boolean plain;
            Clip(String text, String label, boolean plain) { this.text = text; this.label = label; this.plain = plain; }
        }
        final class Access implements ClipboardSnapshots.Access<Clip> {
            Clip current;
            Clip writtenOriginal;
            int writes;
            boolean clearSupported = true, failRead, refuseClear;
            @Override public Clip read() { if (failRead) throw new IllegalStateException("read unavailable"); return current; }
            public boolean canClear() { return clearSupported; }
            @Override public boolean plain(Clip clip) { return clip != null && clip.plain; }
            @Override public String text(Clip clip) { return clip.text; }
            @Override public void write(Clip clip) { writes++; writtenOriginal = clip; if (clip != null || !refuseClear) current = clip; }
            @Override public boolean equal(Clip a, Clip b) { return a.plain && b.plain && a.text.equals(b.text) && a.label.equals(b.label); }
        }
        Access access = new Access(); ClipboardSnapshots<Clip> snapshots = new ClipboardSnapshots<Clip>(2);
        Clip original = new Clip("baseline", "original-label", true);
        access.current = original;
        String id = snapshots.capture(original, access);
        check(id != null && access.writes == 0, "snapshot read-only stores opaque identity");
        check(snapshots.capture(new Clip("uri-or-multi", "rich", false), access) == null && access.writes == 0,
                "unsupported rich baseline cannot grant restore or mutate");
        Scheduler scheduler = new Scheduler(); MainThreadJobs jobs = new MainThreadJobs(scheduler, scheduler, 8);
        access.current = new Clip("sentinel", "riviu", true);
        MainThreadJobs.Job<ClipboardCompare.Result> job = jobs.submit("snapshot_restore_123", ClipboardStore.fingerprint("restoreSnapshot", id, "sentinel"),
                () -> snapshots.restore(id, "sentinel", access), 5, TimeUnit.SECONDS);
        access.current = new Clip("foreign", "human-label", true);
        job.run();
        check(job.result().changed && !job.result().written && access.writes == 0 && "human-label".equals(access.current.label),
                "foreign change while restore queued preserves text and metadata");
        access.current = new Clip("sentinel", "riviu", true);
        ClipboardCompare.Result restored = snapshots.restore(id, "sentinel", access);
        check(restored.written && restored.verified && access.writtenOriginal == original,
                "restore writes original object with original label metadata");
        check("original-label".equals(access.current.label) && "baseline".equals(access.current.text), "restored exact metadata readback");
        access.current = new Clip("sentinel", "rich-uri", false);
        int writes = access.writes;
        ClipboardCompare.Result richChanged = snapshots.restore(id, "sentinel", access);
        check(richChanged.changed && !richChanged.written && access.writes == writes, "same text foreign rich clipboard not overwritten");
        snapshots.capture(new Clip("second", "second", true), access);
        try { snapshots.capture(new Clip("third", "third", true), access); throw new AssertionError("snapshot limit"); }
        catch (IllegalStateException e) { check(snapshots.known(id), "snapshot ledger full never evicts original"); }
        try { snapshots.restore("unknown", "sentinel", access); throw new AssertionError("unknown baseline"); }
        catch (IllegalArgumentException e) { check(access.writes == writes, "unknown snapshot no write"); }
        check(!ClipboardStore.fingerprint("restoreSnapshot", id, "sentinel").equals(ClipboardStore.fingerprint("restoreSnapshot", "other-id", "sentinel")),
                "restore fingerprint binds snapshot ID");
        check(!ClipboardStore.fingerprint("restoreSnapshot", id, "sentinel").equals(ClipboardStore.fingerprint("restoreSnapshot", id, "changed")),
                "restore fingerprint binds expected marker");
        ClipboardSnapshots<Clip> emptySnapshots = new ClipboardSnapshots<Clip>(1);
        check("empty".equals(emptySnapshots.kind(null, access))
                && "plaintext".equals(emptySnapshots.kind(new Clip("", "plain-empty", true), access))
                && "unsupported".equals(emptySnapshots.kind(new Clip("sentinel", "rich", false), access)),
                "successful native read kind distinguishes absent clip from empty string and rich data");
        String emptyId = emptySnapshots.capture(null, access);
        check(emptyId != null && emptySnapshots.known(emptyId), "successful empty baseline retains opaque snapshot identity");
        access.current = new Clip("sentinel", "riviu", true);
        ClipboardCompare.Result emptyRestored = emptySnapshots.restore(emptyId, "sentinel", access);
        check(emptyRestored.written && emptyRestored.verified && access.current == null,
                "empty restore requires exact null readback, never an empty string clip");
        access.current = new Clip("", "riviu", true);
        writes = access.writes;
        check(emptySnapshots.restore(emptyId, "sentinel", access).changed && access.writes == writes,
                "empty string foreign clip is not empty and cannot authorize clear");
        access.current = new Clip("sentinel", "rich", false);
        check(emptySnapshots.restore(emptyId, "sentinel", access).changed && access.writes == writes,
                "same marker in rich clipboard cannot authorize clear");
        access.current = null;
        check(!emptySnapshots.restore(emptyId, "sentinel", access).written && access.writes == writes,
                "missing current marker cannot authorize clear");
        access.current = new Clip("sentinel", "riviu", true);
        access.refuseClear = true;
        check(!emptySnapshots.restore(emptyId, "sentinel", access).verified,
                "clear returning without null readback cannot qualify");
        access.refuseClear = false; access.failRead = true; writes = access.writes;
        try { emptySnapshots.restore(emptyId, "sentinel", access); throw new AssertionError("failed read"); }
        catch (IllegalStateException expected) { check(access.writes == writes, "failed read never grants empty restore"); }
        access.failRead = false; access.clearSupported = false;
        check(new ClipboardSnapshots<Clip>(1).capture(null, access) == null,
                "pre-28 clear unavailable cannot retain restorable empty baseline");
        check(!emptySnapshots.restore(emptyId, "sentinel", access).written && access.writes == writes,
                "lost clear support cannot dispatch clear");
    }
    public static void main(String[] args) throws Exception {
        clipboard(); service(); resources(); bootstrap(); auth(); owner(); binderBootstrap(); acceptDeadline(); compareClipboard(); clipboardSnapshots();
        System.out.println("PASS " + assertions + " behavioral assertions on production Java seams");
    }
}
