package com.riviu.agent;

import java.util.LinkedHashMap;
import java.util.Map;
import java.util.UUID;

/** Instance-local retained originals; bounded, no eviction and no content in identity. */
final class ClipboardSnapshots<T> {
    interface Access<T> {
        T read();
        boolean plain(T clip);
        String text(T clip);
        void write(T original);
        boolean equal(T first, T second);
    }
    private final int limit;
    private final Map<String, T> originals = new LinkedHashMap<String, T>();
    ClipboardSnapshots(int limit) { this.limit = limit; }
    synchronized String capture(T clip, Access<T> access) {
        if (!access.plain(clip)) return null;
        if (originals.size() >= limit) throw new IllegalStateException("clipboard snapshot ledger full");
        String id = UUID.randomUUID().toString();
        originals.put(id, clip);
        return id;
    }
    synchronized boolean known(String id) { return originals.containsKey(id); }
    synchronized ClipboardCompare.Result restore(String id, String expected, Access<T> access) {
        T original = originals.get(id);
        if (original == null) throw new IllegalArgumentException("snapshot_unknown");
        T current = access.read();
        if (current == null) return new ClipboardCompare.Result(false, false, false, false);
        if (!access.plain(current) || !expected.equals(access.text(current))) {
            return new ClipboardCompare.Result(true, false, true, false);
        }
        access.write(original); // Original ClipData object, not rebuilt label/MIME/item metadata.
        T after = access.read();
        return new ClipboardCompare.Result(after != null, true, false, after != null && access.equal(original, after));
    }
}
