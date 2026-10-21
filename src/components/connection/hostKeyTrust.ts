// 第一次连接某台 SSH 服务器时,后端拒绝连接并交回主机密钥:请用户核对指纹,信任后再试一次。
import { getBackend } from "../../lib/api";
import { isCommandError, type HostKey } from "../../lib/types";
import { useUi } from "../../stores/ui";

/** 连续遇到几次「没见过的主机密钥」就不再问了(每次都要用户确认,正常一次就够)。 */
const MAX_PROMPTS = 3;

export function unknownHostKey(error: unknown): HostKey | null {
  return isCommandError(error) && error.code === "host_key_unknown" && error.hostKey ? error.hostKey : null;
}

/** 执行要经 SSH 隧道的操作;遇到没见过的主机密钥就请用户核对。用户取消时返回 null。 */
export async function withHostKeyTrust<T>(action: () => Promise<T>): Promise<T | null> {
  for (let prompts = 0; ; prompts++) {
    try {
      return await action();
    } catch (error) {
      const hostKey = unknownHostKey(error);
      if (!hostKey || prompts >= MAX_PROMPTS) throw error;
      if (!(await useUi.getState().confirmHostKey(hostKey))) return null;
      await (await getBackend()).trustHostKey(hostKey);
    }
  }
}
