#!/usr/bin/env python3
"""Isolated contract tests for bounded DVT app process control."""

from __future__ import annotations

import asyncio
import builtins
import contextlib
import io
import json
import os
import sys
import time
import types
import unittest
from types import SimpleNamespace
from unittest import mock

from sidecars.pymobiledevice3 import riviu_pmd


@contextlib.contextmanager
def app_control_modules(
    pids,
    *,
    delayed=(),
    lookup_error=None,
    cleanup_error=None,
):
    state = SimpleNamespace(
        serial=None,
        process_control=None,
        killed=[],
        lookup_calls=0,
        close_calls=0,
        dvt_exit_calls=0,
        process_exit_calls=0,
    )
    delayed = set(delayed)
    pid_values = iter(pids) if pids is not None else None

    async def maybe_delay(name):
        if name in delayed:
            await asyncio.sleep(1)

    class FakeLockdown:
        async def close(self):
            state.close_calls += 1
            await maybe_delay("lockdown_close")
            if cleanup_error == "lockdown_close":
                raise RuntimeError("cleanup fault")

    async def create_using_usbmux(*, serial):
        state.serial = serial
        await maybe_delay("lockdown_create")
        return FakeLockdown()

    class FakeDvtProvider:
        def __init__(self, lockdown):
            self.lockdown = lockdown

        async def __aenter__(self):
            await maybe_delay("dvt_enter")
            return self

        async def __aexit__(self, *_args):
            state.dvt_exit_calls += 1
            await maybe_delay("dvt_exit")
            if cleanup_error == "dvt_exit":
                raise RuntimeError("cleanup fault")
            return False

    class FakeProcessControl:
        def __init__(self, provider):
            self.provider = provider
            state.process_control = self

        async def __aenter__(self):
            await maybe_delay("process_enter")
            return self

        async def __aexit__(self, *_args):
            state.process_exit_calls += 1
            await maybe_delay("process_exit")
            if cleanup_error == "process_exit":
                raise RuntimeError("cleanup fault")
            return False

        async def process_identifier_for_bundle_identifier(self, bundle_id):
            self.bundle_id = bundle_id
            state.lookup_calls += 1
            boundary = "initial_lookup" if state.lookup_calls == 1 else "poll_lookup"
            await maybe_delay(boundary)
            if lookup_error is not None:
                raise lookup_error
            if pid_values is None:
                return 42
            try:
                return next(pid_values)
            except StopIteration:
                return 42

        async def kill(self, pid):
            await maybe_delay("kill")
            state.killed.append(pid)

    modules = {
        "pymobiledevice3.lockdown": types.SimpleNamespace(
            create_using_usbmux=create_using_usbmux
        ),
        "pymobiledevice3.services.dvt.instruments.dvt_provider": types.SimpleNamespace(
            DvtProvider=FakeDvtProvider
        ),
        "pymobiledevice3.services.dvt.instruments.process_control": types.SimpleNamespace(
            ProcessControl=FakeProcessControl
        ),
    }
    with (
        mock.patch.object(riviu_pmd, "try_import", return_value=True),
        mock.patch.dict(sys.modules, modules),
        mock.patch.object(
            riviu_pmd, "TERMINATE_TIMEOUT_SECONDS", 0.02, create=True
        ),
        mock.patch.object(
            riviu_pmd, "TERMINATE_CLEANUP_TIMEOUT_SECONDS", 0.01, create=True
        ),
        mock.patch.object(riviu_pmd, "TERMINATE_POLL_SECONDS", 0.001, create=True),
    ):
        yield state


class AppControlTests(unittest.TestCase):
    def test_ping_advertises_versioned_process_control_only_when_dependency_is_ready(self):
        for dependency_ready, process_control_ready, expected_code, expected_contracts in [
            (True, True, 0, ["verifiedProcessControl"]),
            (True, False, 0, []),
            (False, False, 2, []),
        ]:
            with self.subTest(
                dependency_ready=dependency_ready,
                process_control_ready=process_control_ready,
            ):
                stdout = io.StringIO()
                with (
                    mock.patch.object(
                        riviu_pmd, "try_import", return_value=dependency_ready
                    ),
                    mock.patch.object(
                        riviu_pmd,
                        "verified_process_control_ready",
                        return_value=process_control_ready,
                        create=True,
                    ),
                    contextlib.redirect_stdout(stdout),
                ):
                    self.assertEqual(
                        riviu_pmd.cmd_ping(SimpleNamespace()), expected_code
                    )
                self.assertEqual(
                    json.loads(stdout.getvalue()),
                    {
                        "ok": True,
                        "pymobiledevice3": dependency_ready,
                        "sidecarProtocolVersion": 2,
                        "contracts": expected_contracts,
                    },
                )

    def test_verified_process_control_readiness_requires_exact_version_and_dvt_classes(self):
        class FakeDvtProvider:
            pass

        class FakeProcessControl:
            pass

        real_import = builtins.__import__

        def dependency_import(name, globals=None, locals=None, fromlist=(), level=0):
            if name == "pymobiledevice3.services.dvt.instruments.dvt_provider":
                return SimpleNamespace(DvtProvider=FakeDvtProvider)
            if name == "pymobiledevice3.services.dvt.instruments.process_control":
                return SimpleNamespace(ProcessControl=FakeProcessControl)
            return real_import(name, globals, locals, fromlist, level)

        with (
            mock.patch("importlib.metadata.version", return_value="10.1.0"),
            mock.patch("builtins.__import__", side_effect=dependency_import),
        ):
            self.assertTrue(riviu_pmd.verified_process_control_ready())

        with mock.patch("importlib.metadata.version", return_value="10.0.0"):
            self.assertFalse(riviu_pmd.verified_process_control_ready())

        def missing_process_control(
            name, globals=None, locals=None, fromlist=(), level=0
        ):
            if name == "pymobiledevice3.services.dvt.instruments.dvt_provider":
                return SimpleNamespace(DvtProvider=FakeDvtProvider)
            if name == "pymobiledevice3.services.dvt.instruments.process_control":
                raise ImportError("missing process control")
            return real_import(name, globals, locals, fromlist, level)

        with (
            mock.patch("importlib.metadata.version", return_value="10.1.0"),
            mock.patch("builtins.__import__", side_effect=missing_process_control),
        ):
            self.assertFalse(riviu_pmd.verified_process_control_ready())

        stderr = io.StringIO()
        with (
            mock.patch("importlib.metadata.version", return_value="10.1.0"),
            mock.patch("builtins.__import__", side_effect=missing_process_control),
            mock.patch.dict(
                os.environ, {riviu_pmd.CONTRACT_DIAGNOSTICS_ENV: "1"}
            ),
            contextlib.redirect_stderr(stderr),
        ):
            self.assertFalse(riviu_pmd.verified_process_control_ready())
        self.assertEqual(
            json.loads(stderr.getvalue()),
            {
                "contract": "verifiedProcessControl",
                "errorType": "ImportError",
                "error": "missing process control",
            },
        )

    def test_verified_terminate_kills_exact_pid_and_observes_absence(self):
        with app_control_modules([42, 0]) as state:
            result = asyncio.run(
                riviu_pmd._terminate_app_verified("fixture", "com.fixture.app")
            )

        self.assertEqual(
            result,
            {
                "ok": True,
                "bundleId": "com.fixture.app",
                "oldPid": 42,
                "running": False,
            },
        )
        self.assertEqual(state.serial, "fixture")
        self.assertEqual(state.killed, [42])

    def test_verified_terminate_accepts_an_already_absent_process(self):
        with app_control_modules([0]) as state:
            result = asyncio.run(
                riviu_pmd._terminate_app_verified("fixture", "com.fixture.app")
            )

        self.assertEqual(
            result,
            {
                "ok": True,
                "bundleId": "com.fixture.app",
                "oldPid": None,
                "running": False,
            },
        )
        self.assertEqual(state.killed, [])

    def test_verified_terminate_times_out_when_the_bundle_stays_running(self):
        started = time.monotonic()
        with app_control_modules(None) as state:
            with self.assertRaises(TimeoutError):
                asyncio.run(
                    riviu_pmd._terminate_app_verified("fixture", "com.fixture.app")
                )
        self.assertLess(time.monotonic() - started, 0.25)
        self.assertEqual(state.killed, [42])

    def test_every_await_boundary_is_bounded(self):
        boundaries = [
            "lockdown_create",
            "dvt_enter",
            "process_enter",
            "initial_lookup",
            "kill",
            "poll_lookup",
            "process_exit",
            "dvt_exit",
            "lockdown_close",
        ]
        for boundary in boundaries:
            with self.subTest(boundary=boundary):
                pids = None if boundary == "poll_lookup" else [42, 0]
                started = time.monotonic()
                with app_control_modules(pids, delayed={boundary}):
                    with self.assertRaises(TimeoutError):
                        asyncio.run(
                            riviu_pmd._terminate_app_verified(
                                "fixture", "com.fixture.app"
                            )
                        )
                self.assertLess(time.monotonic() - started, 0.25)

    def test_cleanup_fault_does_not_replace_the_primary_operation_error(self):
        stderr = io.StringIO()
        with app_control_modules(
            [42],
            lookup_error=RuntimeError("primary operation"),
            cleanup_error="lockdown_close",
        ):
            with contextlib.redirect_stderr(stderr):
                with self.assertRaisesRegex(RuntimeError, "primary operation"):
                    asyncio.run(
                        riviu_pmd._terminate_app_verified(
                            "fixture", "com.fixture.app"
                        )
                    )
        self.assertIn("cleanup fault", stderr.getvalue())

    def test_process_inspection_is_read_only_for_running_and_absent_apps(self):
        for pid in [42, 0]:
            with self.subTest(pid=pid):
                with app_control_modules([pid]) as state:
                    result = asyncio.run(
                        riviu_pmd._inspect_app_process(
                            "fixture", "com.fixture.app"
                        )
                    )
                self.assertEqual(
                    result,
                    {
                        "ok": True,
                        "bundleId": "com.fixture.app",
                        "pid": pid or None,
                        "running": bool(pid),
                    },
                )
                self.assertEqual(state.killed, [])

    def test_invalid_pid_values_fail_closed(self):
        for pid in [True, -1, 0.0, "42"]:
            with self.subTest(pid=pid):
                with app_control_modules([pid]):
                    with self.assertRaises(RuntimeError):
                        asyncio.run(
                            riviu_pmd._inspect_app_process(
                                "fixture", "com.fixture.app"
                            )
                        )

    def test_command_handlers_emit_structured_timeout_errors_and_fail(self):
        args = SimpleNamespace(udid="fixture", bundle_id="com.fixture.app")
        cases = [
            (riviu_pmd.cmd_terminate, [42, 0], {"kill"}),
            (riviu_pmd.cmd_app_process, [42], {"initial_lookup"}),
        ]
        for handler, pids, delayed in cases:
            with self.subTest(handler=handler.__name__):
                stdout = io.StringIO()
                with (
                    app_control_modules(pids, delayed=delayed),
                    contextlib.redirect_stdout(stdout),
                ):
                    self.assertEqual(handler(args), 1)
                self.assertEqual(
                    json.loads(stdout.getvalue()),
                    {
                        "ok": False,
                        "error": "app process-control deadline expired",
                    },
                )


if __name__ == "__main__":
    unittest.main()
