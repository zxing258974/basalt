import { CircleCheck, CircleAlert, FolderOpen } from "lucide-react";
import { useState } from "react";
import { useConnectionActions } from "../../hooks/useConnectionActions";
import { getBackend } from "../../lib/api";
import { errorMessage, isCommandError, type FieldError } from "../../lib/types";
import { useUi, type ConnectionDialogState } from "../../stores/ui";
import { Button } from "../ui/Button";
import { Dialog } from "../ui/Dialog";
import { Checkbox, Field } from "../ui/Field";
import { connectionKey } from "../sidebar/treeKeys";
import { defaultName, emptyForm, formFromConfig, toDraft, withDriver, type ConnectionForm } from "./connectionForm";
import { driverMeta, DRIVERS } from "./driverMeta";
import { DriverPicker } from "./DriverPicker";

type TestResult = { ok: true; message: string } | { ok: false; message: string } | null;

export function ConnectionDialog() {
  const state = useUi((s) => s.connectionDialog);
  if (state.mode === "closed") return null;
  // 每次打开都重新挂载,表单状态自然重置。
  return <OpenConnectionDialog state={state} />;
}

function OpenConnectionDialog({ state }: { state: Exclude<ConnectionDialogState, { mode: "closed" }> }) {
  const close = useUi((s) => s.closeConnectionDialog);
  const [driver, setDriver] = useState(state.mode === "edit" ? state.connection.driver : state.driver);

  return (
    <Dialog
      open
      title={state.mode === "edit" ? "编辑连接" : "新建连接"}
      subtitle={driver ? undefined : "选择要连接的数据库类型"}
      onClose={close}
      width={driver ? 560 : 520}
    >
      {driver ? (
        <ConnectionFormBody
          key={driver}
          initial={state.mode === "edit" ? formFromConfig(state.connection) : emptyForm(driver)}
          onBack={state.mode === "create" ? () => setDriver(null) : undefined}
          onDone={close}
        />
      ) : (
        <DriverPicker onPick={setDriver} />
      )}
    </Dialog>
  );
}

interface BodyProps {
  initial: ConnectionForm;
  onBack?: () => void;
  onDone(): void;
}

function ConnectionFormBody({ initial, onBack, onDone }: BodyProps) {
  const { afterSave } = useConnectionActions();
  const select = useUi((s) => s.select);
  const [form, setForm] = useState(initial);
  const [fieldErrors, setFieldErrors] = useState<FieldError[]>([]);
  const [test, setTest] = useState<TestResult>(null);
  const [busy, setBusy] = useState<"test" | "save" | null>(null);
  const meta = driverMeta(form.driver);
  const isFile = form.driver === "sqlite";

  const update = (patch: Partial<ConnectionForm>) => {
    setForm((current) => ({ ...current, ...patch }));
    setTest(null);
  };
  const errorFor = (field: string) => fieldErrors.find((e) => e.field === field)?.message;

  const run = async (kind: "test" | "save") => {
    setBusy(kind);
    setFieldErrors([]);
    setTest(null);
    try {
      const backend = await getBackend();
      const draft = toDraft(form);
      if (kind === "test") {
        const server = await backend.testConnection(draft);
        setTest({ ok: true, message: `连接成功 · ${server.product} ${server.version}` });
      } else {
        const saved = await backend.saveConnection(draft);
        await afterSave(saved.id);
        select(connectionKey(saved.id));
        onDone();
      }
    } catch (error) {
      if (isCommandError(error) && error.fields.length > 0) setFieldErrors(error.fields);
      else setTest({ ok: false, message: errorMessage(error) });
    } finally {
      setBusy(null);
    }
  };

  const browse = async () => {
    const picked = await (await getBackend()).pickSqliteFile();
    if (picked) update({ path: picked });
  };

  return (
    <form
      className="connection-form"
      onSubmit={(event) => {
        event.preventDefault();
        void run("save");
      }}
    >
      <div className="connection-form-driver">
        <label className="field-label" htmlFor="driver-select">
          类型
        </label>
        <select
          id="driver-select"
          className="select"
          value={form.driver}
          disabled={form.id !== null}
          onChange={(e) => update(withDriver(form, e.target.value as ConnectionForm["driver"]))}
        >
          {DRIVERS.map((d) => (
            <option key={d.kind} value={d.kind}>
              {d.label}
            </option>
          ))}
        </select>
      </div>

      <div className="form-grid">
        <Field
          className="span-4"
          label="连接名"
          value={form.name}
          placeholder={defaultName(form)}
          error={errorFor("name")}
          onValueChange={(name) => update({ name })}
          autoFocus
        />
        <Field label="分组" value={form.group} placeholder="不分组" onValueChange={(group) => update({ group })} />

        {isFile ? (
          <Field
            className="span-full"
            label="数据库文件"
            value={form.path}
            placeholder="/path/to/database.db"
            error={errorFor("path")}
            onValueChange={(path) => update({ path })}
            trailing={
              <Button icon={<FolderOpen />} onClick={() => void browse()}>
                浏览…
              </Button>
            }
          />
        ) : (
          <>
            <Field
              className="span-4"
              label="主机"
              value={form.host}
              error={errorFor("host")}
              onValueChange={(host) => update({ host })}
            />
            <Field
              label="端口"
              value={form.port}
              inputMode="numeric"
              error={errorFor("port")}
              onValueChange={(port) => update({ port })}
            />
            <Field
              label="用户名"
              value={form.username}
              error={errorFor("username")}
              onValueChange={(username) => update({ username })}
            />
            <Field
              label="密码"
              type="password"
              value={form.password}
              placeholder={form.hadSavedPassword && !form.passwordTouched ? "已保存在钥匙串" : ""}
              onValueChange={(password) => update({ password, passwordTouched: true })}
            />
            <Field
              label="初始数据库"
              value={form.database}
              hint={meta.databaseHint}
              onValueChange={(database) => update({ database })}
            />
            <div className="span-full">
              <Checkbox
                label="把密码保存到钥匙串"
                checked={form.savePassword}
                onCheckedChange={(savePassword) => update({ savePassword })}
              />
            </div>
          </>
        )}
      </div>

      {test && (
        <p className={test.ok ? "test-result is-ok" : "test-result is-error"} role="status">
          {test.ok ? <CircleCheck /> : <CircleAlert />}
          <span className="selectable">{test.message}</span>
        </p>
      )}

      <footer className="connection-form-footer">
        {onBack && (
          <Button variant="ghost" onClick={onBack}>
            ← 换类型
          </Button>
        )}
        <Button busy={busy === "test"} disabled={busy !== null} onClick={() => void run("test")}>
          测试连接
        </Button>
        <span className="footer-spacer" />
        <Button variant="ghost" onClick={onDone}>
          取消
        </Button>
        <Button variant="primary" type="submit" busy={busy === "save"} disabled={busy !== null}>
          保存
        </Button>
      </footer>
    </form>
  );
}
