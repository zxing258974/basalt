import { useState } from "react";
import type { SavedQueryRef } from "../../lib/queryTypes";
import { errorMessage, isCommandError } from "../../lib/types";
import { Button } from "../ui/Button";
import { Dialog } from "../ui/Dialog";
import { Field } from "../ui/Field";

interface SaveQueryDialogProps {
  /** 保存到哪里(名字之外):连接、库、schema。 */
  location: Omit<SavedQueryRef, "name">;
  connectionName: string;
  initialName: string;
  onSave(ref: SavedQueryRef, overwrite: boolean): Promise<void>;
  onClose(): void;
}

/** 起名并保存成文件。同名的已经有了先问,确认后才覆盖。 */
export function SaveQueryDialog({ location, connectionName, initialName, onSave, onClose }: SaveQueryDialogProps) {
  const [name, setName] = useState(initialName);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [conflict, setConflict] = useState<string | null>(null);

  const submit = async (overwrite: boolean) => {
    setBusy(true);
    setError(undefined);
    try {
      await onSave({ ...location, name: name.trim() }, overwrite);
    } catch (e) {
      if (isCommandError(e) && e.code === "already_exists") setConflict(name.trim());
      else setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  const where = [connectionName, location.database, location.schema].filter(Boolean).join(" / ");
  const conflicting = conflict !== null && conflict === name.trim();
  return (
    <Dialog
      open
      title="保存查询"
      subtitle={`保存到 ${where}，在对象树的「查询」分组里能找到它。`}
      width={440}
      onClose={busy ? () => {} : onClose}
      footer={
        <>
          <span className="footer-spacer" />
          <Button variant="ghost" disabled={busy} onClick={onClose}>
            取消
          </Button>
          {conflicting ? (
            <Button variant="danger" busy={busy} onClick={() => void submit(true)}>
              覆盖
            </Button>
          ) : (
            <Button variant="primary" busy={busy} disabled={!name.trim()} onClick={() => void submit(false)}>
              保存
            </Button>
          )}
        </>
      }
    >
      <form
        onSubmit={(event) => {
          event.preventDefault();
          if (name.trim() && !busy) void submit(conflicting);
        }}
      >
        <Field
          label="名称"
          value={name}
          autoFocus
          placeholder="如：每日订单统计"
          error={error ?? (conflicting ? `这里已经有叫「${conflict}」的查询了，要覆盖它吗？` : undefined)}
          onValueChange={setName}
        />
      </form>
    </Dialog>
  );
}
