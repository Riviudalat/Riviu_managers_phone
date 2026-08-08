#!/usr/bin/env python3
"""Isolated lifecycle tests for the RT-MMO sidecar backend."""

from __future__ import annotations

import contextlib
import io
import signal
import subprocess
import sys
import tempfile
import unittest
import urllib.request
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

from sidecars.pymobiledevice3 import riviu_pmd


class InstalledMetadataTests(unittest.TestCase):
    def run_inventory(self, apps):
        emitted = []
        filters = []

        class FakeLockdown:
            async def close(self):
                return None

        async def create_using_usbmux(*, serial):
            self.assertEqual(serial, "fixture-udid")
            return FakeLockdown()

        class FakeInstallationProxy:
            def __init__(self, *, lockdown):
                self.lockdown = lockdown

            async def __aenter__(self):
                return self

            async def __aexit__(self, *_args):
                return False

            async def get_apps(self, *, bundle_identifiers):
                filters.append(bundle_identifiers)
                return apps

        modules = {
            "pymobiledevice3.services.installation_proxy": SimpleNamespace(
                InstallationProxyService=FakeInstallationProxy
            ),
            "pymobiledevice3.lockdown": SimpleNamespace(
                create_using_usbmux=create_using_usbmux
            ),
        }
        args = SimpleNamespace(udid="fixture-udid", bundle_id="com.mrph.svc")
        with (
            patch.object(riviu_pmd, "try_import", return_value=True),
            patch.dict(sys.modules, modules),
            patch.object(riviu_pmd, "emit", emitted.append),
        ):
            result = riviu_pmd.cmd_is_installed(args)

        self.assertEqual(result, 0)
        self.assertEqual(filters, [["com.mrph.svc"]])
        return emitted

    def test_is_installed_returns_matching_app_metadata(self):
        emitted = self.run_inventory(
            {
                "com.mrph.svc": {
                    "CFBundleShortVersionString": "1.0",
                    "CFBundleVersion": "1",
                    "ApplicationType": "User",
                    "Path": "/private/var/containers/Bundle/Application/FIXTURE/777wealth.app",
                    "SignerIdentity": "iPhone Distribution: Beijing Fixture",
                }
            }
        )

        self.assertEqual(
            emitted,
            [
                {
                    "ok": True,
                    "installed": True,
                    "bundleId": "com.mrph.svc",
                    "version": "1.0",
                    "build": "1",
                    "applicationType": "User",
                    "path": "/private/var/containers/Bundle/Application/FIXTURE/777wealth.app",
                    "signerIdentity": "iPhone Distribution: Beijing Fixture",
                }
            ],
        )

    def test_is_installed_returns_null_metadata_when_app_is_missing(self):
        emitted = self.run_inventory({})

        self.assertEqual(
            emitted,
            [
                {
                    "ok": True,
                    "installed": False,
                    "bundleId": "com.mrph.svc",
                    "version": None,
                    "build": None,
                    "applicationType": None,
                    "path": None,
                    "signerIdentity": None,
                }
            ],
        )


class CapabilityInspectionTests(unittest.TestCase):
    def test_interaction_inspection_is_metadata_only(self):
        emitted = []
        operations = []

        class FakeLockdown:
            udid = "fixture-udid"
            product_type = "iPhone10,1"
            product_version = "16.7.15"

            async def close(self):
                operations.append("close_lockdown")

        async def create_using_usbmux(*, serial, autopair):
            self.assertEqual(serial, "fixture-udid")
            self.assertFalse(autopair)
            operations.append("open_lockdown")
            return FakeLockdown()

        class FakeInstallationProxy:
            def __init__(self, *, lockdown):
                self.lockdown = lockdown

            async def __aenter__(self):
                operations.append("open_installation_proxy")
                return self

            async def __aexit__(self, *_args):
                operations.append("close_installation_proxy")
                return False

            async def get_apps(self, *, bundle_identifiers):
                operations.append(("get_apps", tuple(bundle_identifiers)))
                return {
                    "com.ss.iphone.ugc.Ame": {
                        "CFBundleShortVersionString": "35.0.0",
                        "CFBundleVersion": "350001",
                    },
                    "com.mrph.svc": {
                        "CFBundleShortVersionString": "1.0",
                        "CFBundleVersion": "1",
                        "CFBundleExecutable": "FixtureRunner",
                        "SignerIdentity": "iPhone Distribution: Fixture",
                    },
                }

        def forbidden(*_args, **_kwargs):
            self.fail("inspection invoked a mutating or runtime operation")

        modules = {
            "pymobiledevice3.services.installation_proxy": SimpleNamespace(
                InstallationProxyService=FakeInstallationProxy
            ),
            "pymobiledevice3.lockdown": SimpleNamespace(
                create_using_usbmux=create_using_usbmux
            ),
        }
        args = SimpleNamespace(
            udid="fixture-udid",
            target_bundle_id="com.ss.iphone.ugc.Ame",
            agent_bundle_id="com.mrph.svc",
            rsd_host=None,
            rsd_port=None,
        )
        with (
            patch.object(riviu_pmd, "try_import", return_value=True),
            patch.dict(sys.modules, modules),
            patch.object(riviu_pmd, "emit", emitted.append),
            patch.object(riviu_pmd.subprocess, "Popen", forbidden),
            patch.object(riviu_pmd.subprocess, "run", forbidden),
            patch.object(
                riviu_pmd,
                "_launch_app_with_environment",
                forbidden,
                create=True,
            ),
            patch.object(riviu_pmd, "_wait_device_port", forbidden, create=True),
            patch.object(riviu_pmd, "_stream_mjpeg", forbidden, create=True),
        ):
            result = riviu_pmd.cmd_inspect_device_capabilities(args)

        self.assertEqual(result, 0)
        self.assertEqual(
            operations,
            [
                "open_lockdown",
                "open_installation_proxy",
                (
                    "get_apps",
                    ("com.ss.iphone.ugc.Ame", "com.mrph.svc"),
                ),
                "close_installation_proxy",
                "close_lockdown",
            ],
        )
        self.assertEqual(
            emitted,
            [
                {
                    "ok": True,
                    "udid": "fixture-udid",
                    "productType": "iPhone10,1",
                    "iosVersion": "16.7.15",
                    "transport": "legacyUsbmuxTransport",
                    "targetApp": {
                        "bundleId": "com.ss.iphone.ugc.Ame",
                        "version": "35.0.0",
                        "build": "350001",
                    },
                    "agentApp": {
                        "bundleId": "com.mrph.svc",
                        "version": "1.0",
                        "build": "1",
                        "executableName": "FixtureRunner",
                        "signerIdentity": "iPhone Distribution: Fixture",
                    },
                }
            ],
        )

    def test_interaction_inspection_reports_the_selected_rsd_transport(self):
        emitted = []
        operations = []

        class FakeRsd:
            udid = "fixture-rsd-udid"
            product_type = "iPhone17,1"
            product_version = "18.2"

            def __init__(self, address):
                self.address = address
                operations.append(("create_rsd", address))

            async def connect(self):
                operations.append("connect_rsd")

            async def close(self):
                operations.append("close_rsd")

        class FakeInstallationProxy:
            def __init__(self, *, lockdown):
                self.lockdown = lockdown

            async def __aenter__(self):
                return self

            async def __aexit__(self, *_args):
                return False

            async def get_apps(self, *, bundle_identifiers):
                operations.append(("get_apps", tuple(bundle_identifiers)))
                return {
                    "com.ss.iphone.ugc.Ame": {
                        "CFBundleShortVersionString": "36.0.0",
                        "CFBundleVersion": "360001",
                    },
                    "com.mrph.svc": {
                        "CFBundleShortVersionString": "1.0",
                        "CFBundleVersion": "1",
                        "CFBundleExecutable": "FixtureRunner",
                        "SignerIdentity": "iPhone Distribution: Fixture",
                    },
                }

        modules = {
            "pymobiledevice3.services.installation_proxy": SimpleNamespace(
                InstallationProxyService=FakeInstallationProxy
            ),
            "pymobiledevice3.remote.remote_service_discovery": SimpleNamespace(
                RemoteServiceDiscoveryService=FakeRsd
            ),
        }
        args = SimpleNamespace(
            udid="fixture-rsd-udid",
            target_bundle_id="com.ss.iphone.ugc.Ame",
            agent_bundle_id="com.mrph.svc",
            rsd_host="fd00::1",
            rsd_port=58783,
        )
        with (
            patch.object(riviu_pmd, "try_import", return_value=True),
            patch.dict(sys.modules, modules),
            patch.object(riviu_pmd, "emit", emitted.append),
        ):
            result = riviu_pmd.cmd_inspect_device_capabilities(args)

        self.assertEqual(result, 0)
        self.assertEqual(
            operations,
            [
                ("create_rsd", ("fd00::1", 58783)),
                "connect_rsd",
                (
                    "get_apps",
                    ("com.ss.iphone.ugc.Ame", "com.mrph.svc"),
                ),
                "close_rsd",
            ],
        )
        self.assertEqual(emitted[0]["transport"], "rsdTransport")
        self.assertEqual(emitted[0]["productType"], "iPhone17,1")

    def test_interaction_inspection_rejects_a_partial_rsd_endpoint(self):
        args = SimpleNamespace(
            udid="fixture-udid",
            target_bundle_id="com.ss.iphone.ugc.Ame",
            agent_bundle_id="com.mrph.svc",
            rsd_host="fd00::1",
            rsd_port=None,
        )
        modules = {
            "pymobiledevice3.services.installation_proxy": SimpleNamespace(
                InstallationProxyService=object
            )
        }
        with (
            patch.object(riviu_pmd, "try_import", return_value=True),
            patch.dict(sys.modules, modules),
            self.assertRaisesRegex(ValueError, "provided together"),
        ):
            riviu_pmd.cmd_inspect_device_capabilities(args)

    def test_interaction_inspection_closes_rsd_when_connect_fails(self):
        operations = []

        class FailingRsd:
            def __init__(self, address):
                operations.append(("create_rsd", address))

            async def connect(self):
                operations.append("connect_rsd")
                raise RuntimeError("fixture connect failure")

            async def close(self):
                operations.append("close_rsd")

        modules = {
            "pymobiledevice3.services.installation_proxy": SimpleNamespace(
                InstallationProxyService=object
            ),
            "pymobiledevice3.remote.remote_service_discovery": SimpleNamespace(
                RemoteServiceDiscoveryService=FailingRsd
            ),
        }
        args = SimpleNamespace(
            udid="fixture-rsd-udid",
            target_bundle_id="com.ss.iphone.ugc.Ame",
            agent_bundle_id="com.mrph.svc",
            rsd_host="fd00::1",
            rsd_port=58783,
        )
        with (
            patch.object(riviu_pmd, "try_import", return_value=True),
            patch.dict(sys.modules, modules),
            self.assertRaisesRegex(RuntimeError, "fixture connect failure"),
        ):
            riviu_pmd.cmd_inspect_device_capabilities(args)

        self.assertEqual(
            operations,
            [
                ("create_rsd", ("fd00::1", 58783)),
                "connect_rsd",
                "close_rsd",
            ],
        )

    def test_interaction_inspection_closes_legacy_provider_when_inventory_fails(self):
        operations = []

        class FakeLockdown:
            udid = "fixture-udid"
            product_type = "iPhone10,1"
            product_version = "16.7.15"

            async def close(self):
                operations.append("close_lockdown")

        async def create_using_usbmux(*, serial, autopair):
            self.assertEqual(serial, "fixture-udid")
            self.assertFalse(autopair)
            operations.append("open_lockdown")
            return FakeLockdown()

        class FailingInstallationProxy:
            def __init__(self, *, lockdown):
                self.lockdown = lockdown

            async def __aenter__(self):
                operations.append("open_installation_proxy")
                return self

            async def __aexit__(self, *_args):
                operations.append("close_installation_proxy")
                return False

            async def get_apps(self, *, bundle_identifiers):
                _ = bundle_identifiers
                raise RuntimeError("fixture inventory failure")

        modules = {
            "pymobiledevice3.services.installation_proxy": SimpleNamespace(
                InstallationProxyService=FailingInstallationProxy
            ),
            "pymobiledevice3.lockdown": SimpleNamespace(
                create_using_usbmux=create_using_usbmux
            ),
        }
        args = SimpleNamespace(
            udid="fixture-udid",
            target_bundle_id="com.ss.iphone.ugc.Ame",
            agent_bundle_id="com.mrph.svc",
            rsd_host=None,
            rsd_port=None,
        )
        with (
            patch.object(riviu_pmd, "try_import", return_value=True),
            patch.dict(sys.modules, modules),
            self.assertRaisesRegex(RuntimeError, "fixture inventory failure"),
        ):
            riviu_pmd.cmd_inspect_device_capabilities(args)

        self.assertEqual(
            operations,
            [
                "open_lockdown",
                "open_installation_proxy",
                "close_installation_proxy",
                "close_lockdown",
            ],
        )


class DependencyContractTests(unittest.TestCase):
    def test_requirements_pin_the_async_dvt_api_used_by_the_sidecar(self):
        requirements = Path(riviu_pmd.__file__).with_name("requirements.txt").read_text(
            encoding="utf-8"
        )

        self.assertIn("pymobiledevice3==10.1.0", requirements.splitlines())

    def test_lock_keeps_a_binary_cryptography_wheel_for_intel_macos(self):
        lock = Path(riviu_pmd.__file__).with_name("requirements-lock.txt").read_text(
            encoding="utf-8"
        )

        self.assertIn("cryptography==48.0.0", lock.splitlines())
        self.assertNotIn("cryptography==49.0.0", lock.splitlines())


class _FakeResponse:
    status = 200

    def __enter__(self):
        return self

    def __exit__(self, *_args):
        return False


class _FakeProcess:
    def __init__(self, command):
        self.command = list(command)
        self.returncode = None
        self._polls = 0
        self.terminated = False
        self.killed = False

    def poll(self):
        self._polls += 1
        if "relay" in self.command and self._polls >= 2:
            self.returncode = 0
        return self.returncode

    def wait(self, timeout=None):
        _ = timeout
        self.returncode = 0
        return 0

    def terminate(self):
        self.terminated = True
        self.returncode = -15

    def kill(self):
        self.killed = True
        self.returncode = -9


class _Harness:
    def __init__(self, port_results, auth_results=None):
        self.port_results = iter(port_results)
        self.auth_results = iter(auth_results or [True])
        self.port_calls = []
        self.popen_commands = []
        self.run_commands = []
        self.http_requests = []
        self.events = []
        self.processes = []
        self.launches = []
        # The XCTest agent receives its token through the child environment, so
        # the environment is part of the contract under test, not an incidental
        # kwarg.
        self.popen_environments = []

    async def wait_device_port(self, udid, port, timeout=45.0):
        self.port_calls.append((udid, port, timeout))
        return next(self.port_results)

    async def device_http_ready(self, udid, port, token, timeout=3.0, header="X-RT-Token"):
        self.events.append(("auth", udid, port, token, timeout, header))
        return next(self.auth_results)

    async def launch_app_with_environment(self, udid, bundle_id, environment):
        self.launches.append((udid, bundle_id, dict(environment)))
        return 1234

    def popen(self, command, **kwargs):
        self.popen_commands.append(list(command))
        self.popen_environments.append(dict(kwargs.get("env") or {}))
        process = _FakeProcess(command)
        self.processes.append(process)
        return process

    def run(self, command, **_kwargs):
        self.run_commands.append(list(command))
        return SimpleNamespace(returncode=0, stdout="", stderr="")

    def urlopen(self, request, timeout=None):
        _ = timeout
        self.http_requests.append(request)
        return _FakeResponse()

    def run_proxy(self, *, port_results=None, restart=False, args=None):
        if port_results is not None:
            self.port_results = iter(port_results)
        if args is None:
            args = SimpleNamespace(
                backend="rt-mmo",
                local_port=18123,
                device_port=8906,
                mjpeg_port=9093,
                token="TEST_TOKEN",
                bundle_id="com.mrph.svc",
                udid="fixture-udid",
                restart_wda=restart,
                bootstrap_only=False,
            )
        with (
            patch.object(riviu_pmd, "_which", return_value="tidevice"),
            patch.object(riviu_pmd, "_wait_device_port", self.wait_device_port),
            patch.object(riviu_pmd, "_device_http_ready", self.device_http_ready),
            patch.object(
                riviu_pmd,
                "_launch_app_with_environment",
                self.launch_app_with_environment,
            ),
            patch.object(riviu_pmd.subprocess, "Popen", self.popen),
            patch.object(riviu_pmd.subprocess, "run", self.run),
            patch.object(
                riviu_pmd,
                "_windows_kill_on_close_job",
                return_value="fixture-job",
            ),
            patch.object(riviu_pmd, "_windows_close_handle"),
            patch.object(riviu_pmd, "emit", self.events.append),
            patch.object(signal, "signal"),
            patch.object(riviu_pmd.time, "sleep"),
            patch.object(urllib.request, "urlopen", self.urlopen),
        ):
            return riviu_pmd.cmd_wda_proxy(args)


class WindowsExecutableDiscoveryTests(unittest.TestCase):
    def test_frozen_runtime_reenters_itself_for_tidevice(self):
        with (
            patch.object(riviu_pmd.sys, "frozen", True, create=True),
            patch.object(riviu_pmd.sys, "executable", "riviu-pmd-fixture"),
        ):
            prefix = riviu_pmd._tidevice_prefix()

        self.assertEqual(prefix, ["riviu-pmd-fixture", "__tidevice"])

    def test_embedded_tidevice_restores_parent_argv(self):
        original = list(riviu_pmd.sys.argv)
        observed = []

        def tidevice_main():
            observed.append(list(riviu_pmd.sys.argv))

        with patch("tidevice.__main__.main", tidevice_main):
            self.assertEqual(riviu_pmd._embedded_tidevice_main(["--version"]), 0)

        self.assertEqual(observed, [["tidevice", "--version"]])
        self.assertEqual(riviu_pmd.sys.argv, original)

    def test_embedded_script_allowlist_is_exact(self):
        self.assertTrue(
            riviu_pmd._is_allowed_embedded_script(
                Path("fixture/sidecars/signer/riviu_signer.py")
            )
        )
        self.assertTrue(
            riviu_pmd._is_allowed_embedded_script(
                Path("fixture/sidecars/wda/build_and_install.py")
            )
        )
        self.assertFalse(
            riviu_pmd._is_allowed_embedded_script(
                Path("fixture/sidecars/wda/prepare_branded_agent.py")
            )
        )

    def test_frozen_embedded_script_sets_runtime_and_restores_parent_state(self):
        original_argv = list(riviu_pmd.sys.argv)
        observed = []
        runtime_key = "RIVIU_EMBEDDED_PYTHON_RUNTIME"

        with tempfile.TemporaryDirectory() as directory:
            script = Path(directory) / "sidecars" / "signer" / "riviu_signer.py"
            script.parent.mkdir(parents=True)
            script.touch()

            def run_path(path, *, run_name):
                observed.append(
                    (
                        path,
                        run_name,
                        list(riviu_pmd.sys.argv),
                        riviu_pmd.os.environ.get(runtime_key),
                    )
                )

            with (
                patch.object(riviu_pmd.sys, "frozen", True, create=True),
                patch.object(riviu_pmd.sys, "executable", "riviu-pmd-fixture"),
                patch.dict(riviu_pmd.os.environ, {runtime_key: "parent-runtime"}),
                patch.object(riviu_pmd.runpy, "run_path", side_effect=run_path),
            ):
                self.assertEqual(
                    riviu_pmd._embedded_script_main(str(script), ["--help"]), 0
                )
                self.assertEqual(
                    riviu_pmd.os.environ.get(runtime_key), "parent-runtime"
                )

        self.assertEqual(
            observed,
            [
                (
                    str(script),
                    "__main__",
                    [str(script), "--help"],
                    "riviu-pmd-fixture",
                )
            ],
        )
        self.assertEqual(riviu_pmd.sys.argv, original_argv)

    def test_background_process_options_hide_windows_console_and_keep_flags(self):
        with patch.object(riviu_pmd.sys, "platform", "win32"):
            options = riviu_pmd._background_process_options(
                {"creationflags": 0x00000200, "cwd": "fixture"}
            )

        self.assertEqual(options["creationflags"], 0x08000200)
        self.assertEqual(options["cwd"], "fixture")

    def test_background_process_options_do_not_add_windows_flags_elsewhere(self):
        with patch.object(riviu_pmd.sys, "platform", "darwin"):
            options = riviu_pmd._background_process_options({"cwd": "fixture"})

        self.assertEqual(options, {"cwd": "fixture"})

    @unittest.skipUnless(sys.platform == "win32", "Windows console contract")
    def test_background_python_child_has_no_console_window(self):
        result = riviu_pmd._background_run(
            [
                sys.executable,
                "-c",
                "import ctypes; print(int(ctypes.windll.kernel32.GetConsoleWindow() or 0))",
            ],
            capture_output=True,
            text=True,
            check=True,
        )

        self.assertEqual(result.stdout.strip(), "0")

    def test_tidevice_is_found_in_user_python_scripts_when_path_misses(self):
        with tempfile.TemporaryDirectory() as raw_dir:
            python_dir = Path(raw_dir) / "Python314"
            scripts_dir = python_dir / "Scripts"
            scripts_dir.mkdir(parents=True)
            executable = scripts_dir / "tidevice.exe"
            executable.touch()

            with (
                patch("shutil.which", return_value=None),
                patch.object(riviu_pmd.sys, "platform", "win32"),
                patch("site.getusersitepackages", return_value=str(python_dir / "site-packages")),
            ):
                found = riviu_pmd._which("tidevice")

        self.assertEqual(found, str(executable))

    @unittest.skipUnless(sys.platform == "win32", "Windows Job Object contract")
    def test_kill_on_close_job_terminates_a_forcibly_orphaned_child(self):
        child = subprocess.Popen(
            [sys.executable, "-c", "import time; time.sleep(30)"],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        job = None
        try:
            job = riviu_pmd._windows_kill_on_close_job(child)
            self.assertIsNotNone(job)

            riviu_pmd._windows_close_handle(job)
            job = None

            child.wait(timeout=5)
            self.assertIsNotNone(child.returncode)
        finally:
            if job is not None:
                riviu_pmd._windows_close_handle(job)
            if child.poll() is None:
                child.kill()
                child.wait(timeout=5)


class RtMmoProxyTests(unittest.TestCase):
    def test_auth_probe_uses_a_protected_route_and_rejects_non_200(self):
        request = riviu_pmd._rt_auth_request("TEST_TOKEN")

        self.assertTrue(request.startswith(b"GET /wda/locked HTTP/1.1\r\n"))
        self.assertIn(b"X-RT-Token: TEST_TOKEN\r\n", request)
        self.assertTrue(riviu_pmd._http_response_is_ok(b"HTTP/1.1 200 OK\r\n"))
        self.assertFalse(
            riviu_pmd._http_response_is_ok(b"HTTP/1.1 401 Unauthorized\r\n")
        )

    def test_regular_app_launch_uses_dvt_without_spawning_tidevice(self):
        launches = []

        async def launch(udid, bundle_id, environment):
            launches.append((udid, bundle_id, environment))
            return 4321

        args = SimpleNamespace(
            udid="fixture-udid",
            bundle_id="com.ss.iphone.ugc.Ame",
        )
        with (
            patch.object(riviu_pmd, "_launch_app_with_environment", launch),
            patch.object(
                riviu_pmd.subprocess,
                "run",
                side_effect=AssertionError("tidevice launch must not be spawned"),
            ),
            patch.object(riviu_pmd, "emit") as emit,
        ):
            self.assertEqual(riviu_pmd.cmd_launch(args), 0)

        self.assertEqual(
            launches,
            [("fixture-udid", "com.ss.iphone.ugc.Ame", {})],
        )
        emit.assert_called_once()

    def test_cold_start_launches_app_with_required_environment(self):
        harness = _Harness([False, True, True])

        self.assertEqual(harness.run_proxy(), 0)

        self.assertEqual(
            harness.launches,
            [
                (
                    "fixture-udid",
                    "com.mrph.svc",
                    {
                        "USE_PORT": "8906",
                        "MJPEG_SERVER_PORT": "9093",
                        "FARM_KEY": "TEST_TOKEN",
                    },
                )
            ],
        )
        self.assertFalse(
            any("TEST_TOKEN" in arg for command in harness.run_commands for arg in command)
        )
        self.assertFalse(any("xctest" in command for command in harness.popen_commands))

    def test_riviu_agent_forwards_text_capability_into_xctest_environment(self):
        # The riviu-agent backend starts the XCTest service rather than issuing a
        # DVT app launch, so the capability and the token travel in the child's
        # environment. Both must stay out of argv: the token is a secret and argv
        # is world-readable on the host.
        harness = _Harness([False, True, True])
        args = SimpleNamespace(
            backend="riviu-agent",
            local_port=18124,
            device_port=8916,
            mjpeg_port=9094,
            token="TEST_TOKEN",
            bundle_id="com.riviu.managersphone.agent.xctrunner",
            udid="fixture-udid",
            restart_wda=False,
            bootstrap_only=False,
        )

        with patch.dict(riviu_pmd.os.environ, {"RIVIU_AGENT_TEXT_CAPABLE": "1"}):
            self.assertEqual(harness.run_proxy(args=args), 0)

        xctest_environments = [
            environment
            for command, environment in zip(
                harness.popen_commands, harness.popen_environments
            )
            if any("xctest" in str(argument) for argument in command)
        ]
        self.assertEqual(len(xctest_environments), 1)
        self.assertEqual(xctest_environments[0]["RIVIU_AGENT_TEXT_CAPABLE"], "1")
        self.assertEqual(xctest_environments[0]["RIVIU_AGENT_TOKEN"], "TEST_TOKEN")
        self.assertEqual(harness.launches, [])
        self.assertFalse(
            any(
                "TEST_TOKEN" in str(argument)
                for command in harness.popen_commands + harness.run_commands
                for argument in command
            )
        )

    def test_live_rtmmo_port_is_reused_without_launch_or_kill(self):
        harness = _Harness([True, True])

        self.assertEqual(harness.run_proxy(), 0)

        self.assertEqual(harness.launches, [])
        self.assertFalse(any("kill" in command for command in harness.run_commands))

    def test_cold_start_retries_launch_once_when_first_bind_probe_fails(self):
        harness = _Harness([False, False, False, True, True])

        self.assertEqual(harness.run_proxy(), 0)

        self.assertEqual(len(harness.launches), 2)
        kills = [command for command in harness.run_commands if "kill" in command]
        self.assertEqual(
            kills,
            [["tidevice", "-u", "fixture-udid", "kill", "com.mrph.svc"]],
        )

    def test_cold_start_rejects_agent_that_binds_but_fails_auth(self):
        # Port probes, in order: the pre-launch "already running?" check, the
        # first launch's wait, the close-poll before the retry, then the retry's
        # wait. The agent only binds on the retry, so the cold-start relaunch is
        # exercised before auth gets to reject it.
        harness = _Harness(
            [False, False, False, True],
            auth_results=[False, False],
        )

        self.assertEqual(harness.run_proxy(), 1)

        relays = [command for command in harness.popen_commands if "relay" in command]
        self.assertEqual(len(harness.launches), 2)
        self.assertEqual(relays, [])
        self.assertFalse(
            any(isinstance(event, dict) and event.get("ok") for event in harness.events)
        )

    def test_cold_start_rejects_control_and_auth_without_mjpeg(self):
        # Same cold-start relaunch as above, but control binds and auth passes;
        # the final probe is the MJPEG port, which never opens.
        harness = _Harness(
            [False, False, False, True, False],
            auth_results=[True, True],
        )

        self.assertEqual(harness.run_proxy(), 1)

        relays = [command for command in harness.popen_commands if "relay" in command]
        self.assertEqual(len(harness.launches), 2)
        self.assertEqual(relays, [])
        self.assertFalse(
            any(isinstance(event, dict) and event.get("ok") for event in harness.events)
        )

    def test_relay_uses_selected_port_and_readiness_request_has_token(self):
        harness = _Harness([True, True])

        self.assertEqual(harness.run_proxy(), 0)

        self.assertIn(
            ["tidevice", "-u", "fixture-udid", "relay", "18123", "8906"],
            harness.popen_commands,
        )
        request = harness.http_requests[0]
        self.assertIsInstance(request, urllib.request.Request)
        self.assertEqual(
            request.full_url,
            "http://127.0.0.1:18123/wda/locked",
        )
        headers = {key.lower(): value for key, value in request.header_items()}
        self.assertEqual(headers.get("x-rt-token"), "TEST_TOKEN")

    def test_control_without_mjpeg_is_relaunched_with_the_full_environment(self):
        harness = _Harness([True, False, False, True, True])

        self.assertEqual(harness.run_proxy(), 0)

        kills = [command for command in harness.run_commands if "kill" in command]
        self.assertEqual(
            kills,
            [["tidevice", "-u", "fixture-udid", "kill", "com.mrph.svc"]],
        )
        self.assertEqual(len(harness.launches), 1)
        self.assertEqual(harness.launches[0][2]["MJPEG_SERVER_PORT"], "9093")

    def test_control_with_wrong_auth_is_relaunched_instead_of_reused(self):
        harness = _Harness([True, True, False, True, True], auth_results=[False, True])

        self.assertEqual(harness.run_proxy(), 0)

        kills = [command for command in harness.run_commands if "kill" in command]
        self.assertEqual(len(kills), 1)
        self.assertEqual(len(harness.launches), 1)
        self.assertEqual(harness.launches[0][2]["FARM_KEY"], "TEST_TOKEN")
        self.assertFalse(
            any("TEST_TOKEN" in arg for command in harness.run_commands for arg in command)
        )

    def test_restart_kills_only_selected_bundle_once(self):
        harness = _Harness([True, False, True, True])

        self.assertEqual(harness.run_proxy(restart=True), 0)

        kills = [command for command in harness.run_commands if "kill" in command]
        self.assertEqual(
            kills,
            [["tidevice", "-u", "fixture-udid", "kill", "com.mrph.svc"]],
        )

    def test_restart_fails_when_the_old_device_port_never_closes(self):
        harness = _Harness([True, True, True, True, True, True, True])

        self.assertEqual(harness.run_proxy(restart=True), 1)

        self.assertEqual(harness.launches, [])
        self.assertTrue(
            any(
                isinstance(event, dict)
                and event.get("ok") is False
                and "did not close" in event.get("error", "")
                for event in harness.events
            )
        )

    def test_bootstrap_only_restarts_agent_without_spawning_a_local_relay(self):
        harness = _Harness([True, False, True, True])
        args = SimpleNamespace(
            backend="rt-mmo",
            local_port=18123,
            device_port=8906,
            mjpeg_port=9093,
            token="TEST_TOKEN",
            bundle_id="com.mrph.svc",
            udid="fixture-udid",
            restart_wda=True,
            bootstrap_only=True,
        )

        self.assertEqual(harness.run_proxy(args=args), 0)

        self.assertFalse(any("relay" in command for command in harness.popen_commands))
        self.assertTrue(
            any(
                isinstance(event, dict)
                and event.get("ok") is True
                and event.get("bootstrapOnly") is True
                for event in harness.events
            )
        )


class StockProxyRegressionTests(unittest.TestCase):
    def test_legacy_stock_arguments_keep_xctest_and_device_port_8100(self):
        harness = _Harness([False, True])
        legacy_args = SimpleNamespace(
            local_port=18123,
            bundle_id="com.riviu.managersphone.agent.xctrunner",
            udid="fixture-udid",
            restart_wda=False,
        )

        self.assertEqual(harness.run_proxy(args=legacy_args), 0)

        self.assertIn(
            [
                "tidevice",
                "-u",
                "fixture-udid",
                "xctest",
                "-B",
                "com.riviu.managersphone.agent.xctrunner",
            ],
            harness.popen_commands,
        )
        self.assertIn(
            ["tidevice", "-u", "fixture-udid", "relay", "18123", "8100"],
            harness.popen_commands,
        )
        self.assertFalse(any("launch" in command for command in harness.run_commands))

    def test_legacy_stream_arguments_default_control_port_to_8100(self):
        captured = []

        async def stream_auto(*args):
            captured.append(args)

        legacy_args = SimpleNamespace(
            udid="fixture-udid",
            fps=5,
            quality=60,
            max_frames=1,
            mode="auto",
            wda_bundle="com.riviu.managersphone.agent.xctrunner",
            mjpeg_port=9100,
        )
        with (
            patch.object(riviu_pmd, "try_import", return_value=True),
            patch.object(riviu_pmd, "_stream_auto", stream_auto),
        ):
            try:
                result = riviu_pmd.cmd_stream(legacy_args)
            except AttributeError as exc:
                self.fail(f"legacy stream arguments lost their default WDA port: {exc}")

        self.assertEqual(result, 0)
        self.assertEqual(captured[0][-1], 8100)


class StreamControlPortTests(unittest.IsolatedAsyncioTestCase):
    async def test_stream_uses_selected_wda_port_while_waiting_for_mjpeg(self):
        port_calls = []

        async def wait_device_port(udid, port, timeout=45.0):
            port_calls.append((udid, port, timeout))
            return len(port_calls) >= 2

        async def stream_mjpeg(*_args):
            return None

        with (
            patch.object(riviu_pmd, "_wait_device_port", wait_device_port),
            patch.object(riviu_pmd, "_stream_mjpeg", stream_mjpeg),
        ):
            try:
                await riviu_pmd._stream_auto(
                    "fixture-udid",
                    5,
                    60,
                    1,
                    "auto",
                    "com.mrph.svc",
                    9093,
                    wda_port=8906,
                )
            except TypeError as exc:
                self.fail(f"stream must accept a selected WDA port: {exc}")

        self.assertEqual([port for _, port, _ in port_calls], [9093, 8906, 9093])

    async def test_long_lived_mjpeg_reconnects_once_after_socket_close(self):
        wait_calls = []
        stream_calls = []

        async def wait_device_port(udid, port, timeout=45.0):
            wait_calls.append((udid, port, timeout))
            return True

        async def stream_mjpeg(*args):
            stream_calls.append(args)
            if len(stream_calls) == 1:
                raise RuntimeError("MJPEG connection closed")

        with (
            patch.object(riviu_pmd, "_wait_device_port", wait_device_port),
            patch.object(riviu_pmd, "_stream_mjpeg", stream_mjpeg),
            patch.object(riviu_pmd, "_free_local_port", side_effect=[18001, 18002]),
        ):
            await riviu_pmd._stream_auto(
                "fixture-udid",
                5,
                60,
                None,
                "mjpeg",
                "com.mrph.svc",
                9093,
                wda_port=8906,
            )

        self.assertEqual(len(stream_calls), 2)
        self.assertEqual([call[1] for call in stream_calls], [18001, 18002])
        self.assertEqual(wait_calls, [("fixture-udid", 9093, 3.0), ("fixture-udid", 9093, 3.0)])


class CliTests(unittest.TestCase):
    def test_interaction_inspection_cli_passes_metadata_selection_only(self):
        captured = []

        def inspect(args):
            captured.append(args)
            return 0

        argv = [
            "riviu_pmd.py",
            "inspect-device-capabilities",
            "--udid",
            "fixture-udid",
            "--agent-bundle-id",
            "com.mrph.svc",
        ]
        with (
            patch.object(sys, "argv", argv),
            patch.object(riviu_pmd, "cmd_inspect_device_capabilities", inspect),
        ):
            result = riviu_pmd.main()

        self.assertEqual(result, 0)
        self.assertEqual(captured[0].target_bundle_id, "com.ss.iphone.ugc.Ame")
        self.assertEqual(captured[0].agent_bundle_id, "com.mrph.svc")
        self.assertIsNone(captured[0].rsd_host)
        self.assertIsNone(captured[0].rsd_port)

    def test_is_installed_cli_passes_the_selected_bundle(self):
        captured = []

        def is_installed(args):
            captured.append(args)
            return 0

        argv = [
            "riviu_pmd.py",
            "is-installed",
            "--udid",
            "fixture-udid",
            "--bundle-id",
            "com.mrph.svc",
        ]
        with (
            patch.object(sys, "argv", argv),
            patch.object(riviu_pmd, "cmd_is_installed", is_installed),
        ):
            result = riviu_pmd.main()

        self.assertEqual(result, 0)
        self.assertEqual(captured[0].bundle_id, "com.mrph.svc")

    def test_wda_proxy_cli_parses_rtmmo_options_and_reads_token_from_environment(self):
        captured = []

        def proxy(args):
            captured.append(args)
            return 0

        argv = [
            "riviu_pmd.py",
            "wda-proxy",
            "--udid",
            "fixture-udid",
            "--backend",
            "rt-mmo",
            "--device-port",
            "8906",
            "--mjpeg-port",
            "9093",
        ]
        with (
            patch.object(sys, "argv", argv),
            patch.dict(riviu_pmd.os.environ, {"RIVIU_RTMMO_TOKEN": "ENV_TOKEN"}),
            patch.object(riviu_pmd, "cmd_wda_proxy", proxy),
            contextlib.redirect_stderr(io.StringIO()),
        ):
            try:
                result = riviu_pmd.main()
            except SystemExit as exc:
                self.fail(f"wda-proxy CLI rejected RT-MMO options: {exc}")

        self.assertEqual(result, 0)
        self.assertEqual(captured[0].backend, "rt-mmo")
        self.assertEqual(captured[0].device_port, 8906)
        self.assertEqual(captured[0].mjpeg_port, 9093)
        self.assertEqual(captured[0].token, "")

    def test_wda_proxy_cli_rejects_token_in_argv(self):
        argv = [
            "riviu_pmd.py",
            "wda-proxy",
            "--udid",
            "fixture-udid",
            "--backend",
            "rt-mmo",
            "--token",
            "SHOULD_NOT_BE_IN_ARGV",
        ]
        with (
            patch.object(sys, "argv", argv),
            contextlib.redirect_stderr(io.StringIO()),
            self.assertRaises(SystemExit),
        ):
            riviu_pmd.main()

    def test_wda_proxy_reads_token_from_environment_when_argv_omits_it(self):
        captured = []

        def proxy(args):
            captured.append(args)
            return 0

        argv = [
            "riviu_pmd.py",
            "wda-proxy",
            "--udid",
            "fixture-udid",
            "--backend",
            "rt-mmo",
        ]
        with (
            patch.object(sys, "argv", argv),
            patch.dict(riviu_pmd.os.environ, {"RIVIU_RTMMO_TOKEN": "ENV_TOKEN"}),
            patch.object(riviu_pmd, "cmd_wda_proxy", proxy),
            contextlib.redirect_stderr(io.StringIO()),
        ):
            result = riviu_pmd.main()

        self.assertEqual(result, 0)
        self.assertEqual(captured[0].token, "")

    def test_stream_cli_parses_selected_wda_port(self):
        captured = []

        def stream(args):
            captured.append(args)
            return 0

        argv = [
            "riviu_pmd.py",
            "stream",
            "--udid",
            "fixture-udid",
            "--wda-port",
            "8906",
        ]
        with (
            patch.object(sys, "argv", argv),
            patch.object(riviu_pmd, "cmd_stream", stream),
            contextlib.redirect_stderr(io.StringIO()),
        ):
            try:
                result = riviu_pmd.main()
            except SystemExit as exc:
                self.fail(f"stream CLI rejected --wda-port: {exc}")

        self.assertEqual(result, 0)
        self.assertEqual(captured[0].wda_port, 8906)


if __name__ == "__main__":
    unittest.main()
