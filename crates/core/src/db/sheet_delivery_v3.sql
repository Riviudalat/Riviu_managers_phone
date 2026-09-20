ALTER TABLE publish_sheet_sync_state ADD COLUMN report_revision INTEGER NOT NULL DEFAULT 0;
ALTER TABLE publish_sheet_sync_state ADD COLUMN report_enabled INTEGER NOT NULL DEFAULT 0;
ALTER TABLE publish_sheet_sync_state ADD COLUMN report_due_at_ms INTEGER GENERATED ALWAYS AS (
  CASE WHEN report_enabled=1 AND report_revision>report_acked_revision THEN
    CASE WHEN report_revision<>report_attempt_revision THEN 0 ELSE report_next_attempt_at_ms END
  END) VIRTUAL;
CREATE INDEX sheet_report_due ON publish_sheet_sync_state(report_due_at_ms,assignment_id)
  WHERE superseded_epoch IS NULL AND report_due_at_ms IS NOT NULL;
CREATE INDEX sheet_canonical_due ON publish_sheet_outbox(next_attempt_at_ms,assignment_id)
  WHERE state<>'sent' AND delivery_target_json IS NOT NULL;

UPDATE publish_sheet_sync_state SET
  report_revision=COALESCE((SELECT a.revision+c.revision FROM publish_assignments a JOIN publish_campaigns c ON c.id=a.campaign_id WHERE a.id=assignment_id),0),
  report_enabled=COALESCE((SELECT json_valid(c.request_json) AND json_extract(c.request_json,'$.sheetEnabled')=1 AND json_extract(c.request_json,'$.sheetDelivery.version')=2 AND json_extract(c.request_json,'$.sheetDelivery.internalReporting')=1 FROM publish_assignments a JOIN publish_campaigns c ON c.id=a.campaign_id WHERE a.id=assignment_id),0);

CREATE TRIGGER sheet_assignment_insert AFTER INSERT ON publish_assignments BEGIN
  INSERT OR IGNORE INTO publish_sheet_sync_state(assignment_id)
    SELECT NEW.id FROM publish_campaigns c WHERE c.id=NEW.campaign_id
      AND json_valid(c.request_json) AND json_extract(c.request_json,'$.sheetDelivery.version')=2 AND json_extract(c.request_json,'$.sheetEnabled')=1;
  UPDATE publish_sheet_sync_state SET report_revision=NEW.revision+(SELECT revision FROM publish_campaigns WHERE id=NEW.campaign_id),
    report_enabled=COALESCE((SELECT json_extract(request_json,'$.sheetDelivery.internalReporting')=1 FROM publish_campaigns WHERE id=NEW.campaign_id),0)
    WHERE assignment_id=NEW.id;
END;
CREATE TRIGGER sheet_assignment_revision AFTER UPDATE OF revision ON publish_assignments BEGIN
  UPDATE publish_sheet_sync_state SET report_revision=NEW.revision+(SELECT revision FROM publish_campaigns WHERE id=NEW.campaign_id) WHERE assignment_id=NEW.id;
END;
CREATE TRIGGER sheet_campaign_revision AFTER UPDATE OF revision,request_json ON publish_campaigns BEGIN
  INSERT OR IGNORE INTO publish_sheet_sync_state(assignment_id)
    SELECT id FROM publish_assignments WHERE campaign_id=NEW.id AND json_valid(NEW.request_json)
      AND json_extract(NEW.request_json,'$.sheetDelivery.version')=2 AND json_extract(NEW.request_json,'$.sheetEnabled')=1;
  UPDATE publish_sheet_sync_state SET report_revision=NEW.revision+(SELECT revision FROM publish_assignments WHERE id=assignment_id),
    report_enabled=COALESCE(json_extract(NEW.request_json,'$.sheetEnabled')=1 AND json_extract(NEW.request_json,'$.sheetDelivery.internalReporting')=1,0)
    WHERE assignment_id IN (SELECT id FROM publish_assignments WHERE campaign_id=NEW.id);
END;
CREATE TRIGGER sheet_outbox_insert AFTER INSERT ON publish_sheet_outbox WHEN NEW.delivery_target_json IS NOT NULL BEGIN
  INSERT OR IGNORE INTO publish_sheet_sync_state(assignment_id) VALUES(NEW.assignment_id);
END;
CREATE TRIGGER sheet_outbox_target AFTER UPDATE OF delivery_target_json ON publish_sheet_outbox WHEN NEW.delivery_target_json IS NOT NULL BEGIN
  INSERT OR IGNORE INTO publish_sheet_sync_state(assignment_id) VALUES(NEW.assignment_id);
END;
CREATE TABLE publish_sheet_receipts (
  publication_id TEXT NOT NULL, revision INTEGER NOT NULL,
  spreadsheet_id TEXT NOT NULL, sheet_gid INTEGER NOT NULL, epoch TEXT NOT NULL,
  receipt_json TEXT NOT NULL CHECK(json_valid(receipt_json)), observed_at TEXT NOT NULL,
  PRIMARY KEY(publication_id,revision,spreadsheet_id,sheet_gid,epoch)
);

CREATE VIEW sheet_target_epochs AS
SELECT t.assignment_id,e.epoch FROM (
  SELECT assignment_id,delivery_target_json AS target FROM publish_sheet_outbox WHERE delivery_target_json IS NOT NULL
  UNION ALL
  SELECT a.id,json_extract(c.request_json,'$.sheetDelivery') FROM publish_assignments a JOIN publish_campaigns c ON c.id=a.campaign_id
    WHERE json_valid(c.request_json) AND json_extract(c.request_json,'$.sheetDelivery.version')=2
      AND NOT EXISTS(SELECT 1 FROM publish_sheet_outbox o WHERE o.assignment_id=a.id AND o.delivery_target_json IS NOT NULL)
) t JOIN publish_reporting_epochs e ON e.spreadsheet_id=json_extract(t.target,'$.spreadsheetId')
  AND e.sheet_gid=json_extract(t.target,'$.sheetGid')
  WHERE e.paused=0 AND e.epoch<>COALESCE(json_extract(t.target,'$.reportingEpoch'),'legacy');
CREATE TRIGGER sheet_new_state_epoch AFTER INSERT ON publish_sheet_sync_state BEGIN
  UPDATE publish_sheet_sync_state SET superseded_epoch=COALESCE(NEW.superseded_epoch,(SELECT epoch FROM sheet_target_epochs WHERE assignment_id=NEW.assignment_id))
    WHERE assignment_id=NEW.assignment_id;
END;
CREATE TRIGGER sheet_changed_target_epoch AFTER UPDATE OF delivery_target_json ON publish_sheet_outbox
WHEN NEW.delivery_target_json IS NOT OLD.delivery_target_json BEGIN
  UPDATE publish_sheet_sync_state SET superseded_epoch=CASE
    WHEN superseded_epoch=json_extract(NEW.delivery_target_json,'$.reportingEpoch')
      AND EXISTS(SELECT 1 FROM publish_reporting_epochs e WHERE e.paused=0
        AND e.epoch=superseded_epoch AND e.spreadsheet_id=json_extract(NEW.delivery_target_json,'$.spreadsheetId')
        AND e.sheet_gid=json_extract(NEW.delivery_target_json,'$.sheetGid')) THEN NULL
    ELSE COALESCE((SELECT epoch FROM sheet_target_epochs WHERE assignment_id=NEW.assignment_id),superseded_epoch) END
    WHERE assignment_id=NEW.assignment_id AND claim_token IS NULL;
END;
