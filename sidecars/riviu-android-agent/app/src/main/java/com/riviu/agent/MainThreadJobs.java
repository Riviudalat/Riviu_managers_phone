package com.riviu.agent;

import java.util.LinkedHashMap;
import java.util.Map;
import java.util.UUID;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;

/** Production dispatch/settlement seam. Cancellation only claims jobs not yet running. */
final class MainThreadJobs {
    enum State { QUEUED, RUNNING, SUCCEEDED, FAILED, CANCELLED }
    interface Scheduler { boolean post(Runnable task); void remove(Runnable task); }
    interface Action<T> { T run(); }
    interface Clock { long now(); }
    static final class Pending extends IllegalStateException {
        final String operationId;
        final State state;
        Pending(Job<?> job) {
            super(job.state() == State.CANCELLED ? "clipboard_cancelled" : "settlement_required");
            operationId = job.id;
            state = job.state();
        }
    }

    static final class Job<T> implements Runnable {
        final String id;
        final boolean named;
        final String fingerprint;
        private final Scheduler scheduler;
        private final Clock clock;
        private final long deadline;
        private final CountDownLatch done = new CountDownLatch(1);
        private Action<T> action;
        private State state = State.QUEUED;
        private T result;
        private RuntimeException error;

        Job(String id, boolean named, String fingerprint, Scheduler scheduler, Clock clock,
                long timeoutNanos, Action<T> action) {
            this.id = id;
            this.named = named;
            this.fingerprint = fingerprint;
            this.scheduler = scheduler;
            this.clock = clock;
            this.deadline = clock.now() + timeoutNanos;
            this.action = action;
        }
        synchronized State state() { return state; }
        synchronized T result() { return result; }
        synchronized boolean terminal() { return state != State.QUEUED && state != State.RUNNING; }
        boolean cancelQueued() {
            synchronized (this) {
                if (state != State.QUEUED) return false;
                state = State.CANCELLED;
                action = null;
                done.countDown();
            }
            scheduler.remove(this);
            return true;
        }
        void rejected() {
            synchronized (this) {
                if (state != State.QUEUED) return;
                state = State.FAILED;
                action = null;
                error = new IllegalStateException("main-thread dispatch unavailable");
                done.countDown();
            }
        }
        @Override public void run() {
            final Action<T> dispatch;
            synchronized (this) {
                if (state != State.QUEUED) return;
                // Even if the waiting worker has not been scheduled, an expired dispatch cannot write.
                if (clock.now() - deadline >= 0) {
                    state = State.CANCELLED;
                    action = null;
                    done.countDown();
                    return;
                }
                state = State.RUNNING;
                dispatch = action;
            }
            T value = null;
            RuntimeException failure = null;
            try { value = dispatch.run(); }
            catch (RuntimeException e) { failure = e; }
            catch (Error e) { failure = new IllegalStateException("clipboard dispatch failed"); throw e; }
            finally {
                synchronized (this) {
                    result = value;
                    error = failure;
                    action = null;
                    state = failure == null ? State.SUCCEEDED : State.FAILED;
                    done.countDown();
                }
            }
        }
        T await(long timeout, TimeUnit unit) {
            try {
                if (!done.await(timeout, unit)) cancelQueued();
            } catch (InterruptedException e) {
                cancelQueued();
                Thread.currentThread().interrupt();
            }
            synchronized (this) {
                if (state == State.SUCCEEDED) return result;
                if (state == State.FAILED) throw error;
                throw new Pending(this);
            }
        }
    }

    private final Map<String, Job<?>> jobs = new LinkedHashMap<String, Job<?>>();
    private final int limit;
    private final Scheduler scheduler;
    private final Clock clock;
    private boolean closed;
    MainThreadJobs(Scheduler scheduler, Clock clock, int limit) {
        this.scheduler = scheduler;
        this.clock = clock;
        this.limit = limit;
    }
    synchronized <T> Job<T> submit(Action<T> action, long timeout, TimeUnit unit) {
        return submit(null, null, action, timeout, unit);
    }
    @SuppressWarnings("unchecked")
    synchronized <T> Job<T> submit(String id, String fingerprint, Action<T> action,
            long timeout, TimeUnit unit) {
        if (id != null && jobs.containsKey(id)) {
            Job<?> existing = jobs.get(id);
            if (!existing.named || !existing.fingerprint.equals(fingerprint)) {
                throw new IllegalArgumentException("operation_conflict");
            }
            return (Job<T>) existing;
        }
        if (closed) throw new IllegalStateException("clipboard admission closed");
        if (jobs.size() >= limit) {
            java.util.Iterator<Job<?>> iterator = jobs.values().iterator();
            while (iterator.hasNext() && jobs.size() >= limit) {
                Job<?> prior = iterator.next();
                if (!prior.named && prior.terminal()) iterator.remove();
            }
        }
        if (jobs.size() >= limit) throw new IllegalStateException("clipboard ledger full");
        Job<T> job = new Job<T>(id == null ? UUID.randomUUID().toString() : id,
                id != null, fingerprint, scheduler, clock, unit.toNanos(timeout), action);
        jobs.put(job.id, job);
        if (!scheduler.post(job)) job.rejected();
        return job;
    }
    synchronized Job<?> find(String id) { return jobs.get(id); }
    synchronized int unsettled() {
        int count = 0;
        for (Job<?> job : jobs.values()) if (!job.terminal()) count++;
        return count;
    }
    synchronized void closeAdmission() {
        closed = true;
        for (Job<?> job : jobs.values()) job.cancelQueued();
    }
}
