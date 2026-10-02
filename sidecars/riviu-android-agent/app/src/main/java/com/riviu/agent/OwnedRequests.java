package com.riviu.agent;

import java.io.Closeable;
import java.io.IOException;
import java.util.ArrayList;
import java.util.HashSet;
import java.util.List;
import java.util.Set;
import java.util.concurrent.ArrayBlockingQueue;
import java.util.concurrent.RejectedExecutionException;
import java.util.concurrent.ThreadPoolExecutor;
import java.util.concurrent.TimeUnit;

/** Bounded accepted resource ownership. Stop closes sockets but never interrupts running effects. */
final class OwnedRequests {
    private final ThreadPoolExecutor pool;
    private final Set<Task> owned = new HashSet<Task>();
    private boolean closing;
    OwnedRequests(int workers, int queued) {
        pool = new ThreadPoolExecutor(workers, workers, 0, TimeUnit.MILLISECONDS,
                new ArrayBlockingQueue<Runnable>(queued), r -> new Thread(r, "riviu-helper-worker"),
                new ThreadPoolExecutor.AbortPolicy());
    }
    synchronized boolean accept(Closeable resource, Runnable action) {
        Task task = new Task(resource, action);
        if (closing) { task.close(); return false; }
        owned.add(task);
        try { pool.execute(task); return true; }
        catch (RejectedExecutionException e) { owned.remove(task); task.close(); return false; }
    }
    synchronized void closeAdmission() {
        if (closing) return;
        closing = true;
        // No interrupt/shutdownNow: an effect already running must report its actual settlement.
        pool.shutdown();
        List<Runnable> queued = new ArrayList<Runnable>();
        pool.getQueue().drainTo(queued);
        for (Runnable task : queued) { owned.remove(task); ((Task) task).close(); }
        for (Task task : owned) task.close();
    }
    boolean awaitDrain(long timeout, TimeUnit unit) throws InterruptedException {
        return pool.awaitTermination(timeout, unit);
    }
    synchronized int ownedCount() { return owned.size(); }
    private final class Task implements Runnable {
        final Closeable resource;
        final Runnable action;
        Task(Closeable resource, Runnable action) { this.resource = resource; this.action = action; }
        @Override public void run() {
            try {
                synchronized (OwnedRequests.this) { if (closing) return; }
                action.run();
            } finally {
                close();
                synchronized (OwnedRequests.this) { owned.remove(this); }
            }
        }
        void close() { try { resource.close(); } catch (IOException ignored) {} }
    }
}
