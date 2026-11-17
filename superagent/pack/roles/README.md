# roles/ — 角色文件结构规范（P1-1 定义，内容由 P1-2 #74 交付）

每角色一个 `<role>.toml`，对应引擎 `[agents.<role>]` 声明 + 独立 config 层：

- `description`：角色职责一句话（进 leader 的工具目录）。
- `config_file`：指向本目录内该角色的 config 层文件（模型、沙箱、developer_instructions、交付契约）。
- `nickname_candidates`：昵称候选列表（spawn 时实例化）。

约束：角色级 sandbox_mode 不被引擎应用（P0-2 实证）；沙箱兜底由编排器经
command/exec 显式 SandboxPolicy 承担（P0-7）。角色文件不得携带凭据。