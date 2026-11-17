## 背景
ext/goal 提供长期目标+token 预算+steering，是元智能体"任务使命"原语；ext/queue 提供空闲派发。

## 任务
- [ ] create_goal/get_goal/update_goal 工具与 thread/goal/{set,get,clear} API 往返
- [ ] token 预算记账与超限行为；steering 注入
- [ ] ext/queue 空闲派发（thread/queue/*）一次完整流程

## 验收
goal 创建→预算记账→更新/清除 trace；队列空闲派发 trace。

## 代码
ext/goal/；ext/queue/。依赖 #64。

