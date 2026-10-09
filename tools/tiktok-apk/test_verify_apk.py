"""APK identity checks on synthetic aapt2/apksigner output."""

import unittest

import verify_apk

BADGING = "package: name='com.zhiliaoapp.musically' versionCode='2024507030' versionName='45.7.3' platformBuildVersionName='16'\n"
SIGNER = verify_apk.KNOWN_SIGNERS["com.zhiliaoapp.musically"]
APKSIGNER = f"""Verifies
Verified using v1 scheme (JAR signing): true
Verified using v2 scheme (APK Signature Scheme v2): true
Verified using v3.1 scheme (APK Signature Scheme v3.1): false
Number of signers: 1
Signer #1 certificate DN: CN=musical.ly, OU=android
Signer #1 certificate SHA-256 digest: {SIGNER}
Source Stamp Signer certificate SHA-256 digest: {"0" * 64}
"""


class VerifyApkTests(unittest.TestCase):
    def identity(self, signer_text=APKSIGNER):
        return {**verify_apk.parse_badging(BADGING), "signature": verify_apk.parse_apksigner(signer_text)}

    def test_known_build_passes_and_parses_every_field(self):
        identity = self.identity()
        self.assertEqual((identity["package"], identity["versionName"], identity["versionCode"]),
                         ("com.zhiliaoapp.musically", "45.7.3", "2024507030"))
        self.assertEqual(identity["signature"]["schemes"], {"v1": True, "v2": True, "v3.1": False})
        self.assertEqual(identity["signature"]["signers"], [{"sha256": SIGNER, "dn": "CN=musical.ly, OU=android"}])
        self.assertEqual(verify_apk.problems(identity, {"version": "45.7.3", "version_code": "2024507030"}), [])

    def test_foreign_signer_failed_verification_and_wrong_version_are_reported(self):
        foreign = self.identity(APKSIGNER.replace(SIGNER, "1" * 64).replace("Verifies\n", ""))
        found = verify_apk.problems(foreign, {"version": "45.7.4"})
        self.assertEqual(len(found), 3, found)
        self.assertTrue(any("Verifies" in item for item in found))
        self.assertTrue(any("1" * 64 in item for item in found))
        self.assertTrue(any("45.7.4" in item for item in found))

    def test_unknown_package_needs_an_explicit_signer(self):
        identity = self.identity()
        identity["package"] = "com.example.other"
        self.assertEqual(len(verify_apk.problems(identity, {})), 1)
        self.assertEqual(verify_apk.problems(identity, {"signer": SIGNER.upper()}), [])


if __name__ == "__main__":
    unittest.main()
