# templates/ — 模板清单

- `run-record.json`：run 版本绑定记录示例（P1-1 #73）。run 固定 bundle/workflow digest；
  恢复用原 pin 的包，缺失或不符显式阻塞，不静默换规则。
- `acceptance-criteria.json`：验收标准模板（P1-3 #75）。每节点门禁引用 command_source
  + artifact schema；节点成功仅当门禁命令 exit 0 且产物 schema 校验通过；缺门禁命令
  → waiting_input，绝不以模型自报判成功。权威 wire schema 以 P2-1 #78 为准。
- `handoff-contract.md`：节点交接契约（P1-3 #75）。artifact 引用 + 有界摘要 ≤800 tokens；
  低信任产物 user-level，不提权到 developer_instructions。
- 报告 artifact 模板随 #77 交付。