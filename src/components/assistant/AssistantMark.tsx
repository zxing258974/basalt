import { Bot } from "lucide-react";

/** SQL 助手的标志:渐变底色上的白色机器人,和一排灰色的工具按钮一眼分得开。 */
export function AssistantMark() {
  return (
    <span className="assistant-mark" aria-hidden>
      <Bot />
    </span>
  );
}
