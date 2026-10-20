import { useState } from "react";
import { useConnectionActions } from "../../hooks/useConnectionActions";
import { errorMessage } from "../../lib/types";
import { useUi, type PasswordPromptState } from "../../stores/ui";
import { Button } from "../ui/Button";
import { Dialog } from "../ui/Dialog";
import { Checkbox, Field } from "../ui/Field";
import { endpointSummary } from "./driverMeta";

export function PasswordPrompt() {
  const prompt = useUi((s) => s.passwordPrompt);
  if (!prompt) return null;
  return <PasswordPromptDialog key={prompt.connection.id} prompt={prompt} />;
}

function PasswordPromptDialog({ prompt }: { prompt: PasswordPromptState }) {
  const { askPassword, notify } = useUi.getState();
  const { open, rememberPassword } = useConnectionActions();
  const [password, setPassword] = useState("");
  const { connection } = prompt;
  // 连接本来就设为保存密码(比如导入后钥匙串里还没有)时默认记住,免得每次打开都先失败一次。
  const [remember, setRemember] = useState(connection.savePassword);
  const [busy, setBusy] = useState(false);
  /** 认证失败由 open() 重新调用 askPassword 带回 prompt.error;其他错误记在这里。 */
  const [otherError, setOtherError] = useState<string>();

  const submit = async () => {
    setBusy(true);
    setOtherError(undefined);
    askPassword({ connection });
    try {
      const opened = await open({ ...connection, isOpen: false }, password);
      if (!opened) return;
      askPassword(null);
      // 先确认密码能用再记住。记住失败不影响已经打开的连接,只提示。
      if (remember) {
        await rememberPassword(connection.id, password).catch((error: unknown) =>
          notify(`已连接，但保存密码失败：${errorMessage(error)}`),
        );
      }
    } catch (error) {
      setOtherError(errorMessage(error));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog
      open
      title={`连接到「${connection.name}」`}
      subtitle={<span className="mono">{endpointSummary(connection)}</span>}
      width={400}
      onClose={() => askPassword(null)}
    >
      <form
        className="stack"
        onSubmit={(event) => {
          event.preventDefault();
          void submit();
        }}
      >
        <Field
          label="密码"
          type="password"
          value={password}
          error={otherError ?? prompt.error}
          onValueChange={setPassword}
          autoFocus
        />
        <Checkbox label="连接成功后把密码保存到钥匙串" checked={remember} onCheckedChange={setRemember} />
        <footer className="connection-form-footer">
          <span className="footer-spacer" />
          <Button variant="ghost" onClick={() => askPassword(null)}>
            取消
          </Button>
          <Button variant="primary" type="submit" busy={busy}>
            连接
          </Button>
        </footer>
      </form>
    </Dialog>
  );
}
