#!/usr/bin/env python3
"""Build the reviewed 158-row clean-room parity matrix.

The command names are provenance identifiers only.  They are deliberately kept out of
runtime code and are never exposed by the operator UI.
"""

from __future__ import annotations

import argparse
import csv
from pathlib import Path


COMMANDS = """
action_act action_play activate adb_command app_install button2_qrcode button3_qrcode
calibrate_mouse check_adb_device check_auto_update check_brand_stop check_client_token
check_hid_app_installed check_serial_number check_service client_event_reporting client_login
client_logout client_pwd_reset client_refund client_refund_reason client_register close_device
cloud_phone_active_page cloud_phone_connect cloud_phone_get_renew_order_info
cloud_phone_lease_record_page cloud_phone_my_phones cloud_phone_pay_record_page
cloud_phone_renew_prepay cloud_phone_tag_add cloud_phone_tag_del cloud_phone_tag_edit
cloud_phone_tag_list cloud_phone_tag_sort cloud_virtual_active_page cloud_virtual_ali_command
cloud_virtual_auth_device_page cloud_virtual_baidu_command cloud_virtual_blade_command
cloud_virtual_create_ali_lease_device cloud_virtual_lease_record_page
cloud_virtual_model_first_list cloud_virtual_new_brand_product_list
cloud_virtual_pay_record_device_page cloud_virtual_pay_record_page
cloud_virtual_pay_record_renew_info cloud_virtual_voucher_record_page common_request copy cur_env
device_is_accessible disconnect download_image exec_autojs_check gather_coord
get_activation_pay_list get_apk_info get_apk_list get_arp_out get_brand_cloud_service
get_brand_owner get_brand_price_list get_clipboard get_computer_id get_computer_name
get_cs_qr_code_list get_cur_time get_density get_device_info get_device_list get_device_mode
get_ime_info get_ime_list get_ip_serial get_pay_result get_root get_serial_ip get_size
get_system_config get_usb_devices get_windows_language get_ws_port has_hid_devices id2_verify
input_enter input_text install_apk install_hid_app install_input install_magisk install_xwdb
is_duration_greater_than_24_hours is_hid_model launch_app lock_screen merge_adb_auth_key
mouse_button2 mouse_button3 mouse_reset oem_adv_config open_api_ws_connect otg_all_scanning
otg_scanning paste paste_pwd paste_text pay_receipt pull_clipboard pull_pasteboard push_file
push_pasteboard push_scan_ips put_clipboard query_virtual_voucher
query_virtual_voucher_renew_device read_config read_sys_config reboot reboot_ext reconnect_device
remove_hid_driver replace_device replace_record_detail_page replace_record_page restart restart_adb
save_uploaded_file send_sms_verify_code send_transfer_sms_verify_code send_verify_code
set_activation set_auto_mirror set_state start_updater stop_autojs stop_gather_coord
switch_accessible_mode switch_adb_mode switch_all_device_mode switch_direction switch_hid_model
switch_ime sync_client_stat transfer_device transfer_record_detail_page transfer_record_page
un_pre_lock uninstall_apk upload_pre_sign_list usb_to_tcp verify_sms_code version_notice
virtual_replace_status voucher_bind_new_virtual voucher_renew_virtual wallpapers_device write_config
""".split()

IMPLEMENT = {
    "app_install",
    "get_apk_info",
    "get_apk_list",
    "install_apk",
}

COMMERCIAL = {
    "activate",
    "button2_qrcode",
    "button3_qrcode",
    "check_auto_update",
    "check_brand_stop",
    "check_client_token",
    "check_service",
    "client_event_reporting",
    "client_login",
    "client_logout",
    "client_pwd_reset",
    "client_refund",
    "client_refund_reason",
    "client_register",
    "common_request",
    "get_activation_pay_list",
    "get_brand_cloud_service",
    "get_brand_owner",
    "get_brand_price_list",
    "get_cs_qr_code_list",
    "get_cur_time",
    "get_pay_result",
    "id2_verify",
    "is_duration_greater_than_24_hours",
    "oem_adv_config",
    "pay_receipt",
    "replace_device",
    "replace_record_detail_page",
    "replace_record_page",
    "send_sms_verify_code",
    "send_transfer_sms_verify_code",
    "send_verify_code",
    "set_activation",
    "start_updater",
    "sync_client_stat",
    "transfer_device",
    "transfer_record_detail_page",
    "transfer_record_page",
    "upload_pre_sign_list",
    "verify_sms_code",
    "version_notice",
    "virtual_replace_status",
    "voucher_bind_new_virtual",
    "voucher_renew_virtual",
}

SECURITY = {
    "adb_command",
    "calibrate_mouse",
    "check_hid_app_installed",
    "device_is_accessible",
    "exec_autojs_check",
    "has_hid_devices",
    "install_hid_app",
    "install_input",
    "install_magisk",
    "install_xwdb",
    "is_hid_model",
    "merge_adb_auth_key",
    "mouse_button2",
    "mouse_button3",
    "mouse_reset",
    "open_api_ws_connect",
    "otg_all_scanning",
    "otg_scanning",
    "remove_hid_driver",
    "stop_autojs",
    "switch_accessible_mode",
    "switch_hid_model",
    "usb_to_tcp",
}

NOT_APPLICABLE = {
    "download_image",
    "get_computer_id",
    "get_computer_name",
    "get_windows_language",
    "get_ws_port",
    "read_config",
    "read_sys_config",
    "save_uploaded_file",
    "set_state",
    "write_config",
}


def status_for(command: str) -> tuple[str, str, str]:
    if command.startswith(("cloud_phone_", "cloud_virtual_", "query_virtual_")):
        return "commercial-excluded", "none", "account, payment, or cloud-phone surface"
    if command in COMMERCIAL:
        return "commercial-excluded", "none", "commercial account, branding, telemetry, or updater surface"
    if command in SECURITY:
        return "security-excluded", "none", "unsafe vendor transport, privilege, key, script, or HID surface"
    if command in IMPLEMENT:
        return "implement", "Android app library", "clean-room package metadata and controlled install"
    if command in NOT_APPLICABLE:
        return "not-applicable", "none", "host shell detail with no product-level parity requirement"
    if command in {"action_act", "action_play"}:
        return "existing", "Flow and macro runtime", "typed local automation; AutoSwipe extends this surface"
    return "existing", "Riviu device control", "equivalent typed capability already exists"


def build(output: Path) -> None:
    if len(COMMANDS) != 158 or len(set(COMMANDS)) != 158:
        raise RuntimeError("parity inventory must contain exactly 158 unique commands")
    output.parent.mkdir(parents=True, exist_ok=True)
    with output.open("w", encoding="utf-8", newline="") as handle:
        writer = csv.writer(handle, lineterminator="\n")
        writer.writerow(("index", "provenanceCommand", "status", "riviuMapping", "decision"))
        for index, command in enumerate(COMMANDS, 1):
            status, mapping, decision = status_for(command)
            writer.writerow((index, command, status, mapping, decision))


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    build(args.output)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
