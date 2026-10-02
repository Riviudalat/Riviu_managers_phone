package com.riviu.agent;

/** Admission shared by the release Binder carrier and its package-pinned bridge. */
final class BootstrapBinderPolicy {
    static final String PACKAGE = "com.riviu.agent";
    static final String SERVICE = PACKAGE + ".AgentService";
    static final String ACTION = PACKAGE + ".BOOTSTRAP_BINDER";
    static final String DESCRIPTOR = PACKAGE + ".BootstrapBinder.v1";
    static final int PROBE = 1;
    static final int REQUEST = 2;
    static boolean callerAllowed(int kernelUid, boolean dumpGranted) {
        return kernelUid == 2000 && dumpGranted; // Root is explicitly excluded.
    }
    static boolean identityAllowed(String packageName, String serviceName, String permission,
            int actualUid, int checkedUid, String certificate, String pinnedCertificate,
            boolean enabled, boolean exported) {
        return PACKAGE.equals(packageName) && SERVICE.equals(serviceName)
                && "android.permission.DUMP".equals(permission) && enabled && exported
                && BootstrapEnvelope.serverUidAllowed(actualUid, checkedUid)
                && pinnedCertificate != null && pinnedCertificate.matches("[a-fA-F0-9]{64}")
                && pinnedCertificate.equalsIgnoreCase(certificate);
    }
    static boolean sessionMatches(String action, String expectedInstance, String expectedGeneration,
            String actualInstance, String actualGeneration) {
        if ("claim".equals(action) && "-".equals(expectedInstance) && "-".equals(expectedGeneration)) {
            // Lost claim ACK: OwnerSession still requires the same owner AND token, never rekeys.
            return true;
        }
        return actualInstance != null && actualGeneration != null
                && actualInstance.equals(expectedInstance) && actualGeneration.equals(expectedGeneration);
    }
}
