# Instruction

## Project Alpha

现在mcp的rust语言的sdk库: rmcp ,最新版本是: rmcp-v0.12.0 ,但是从版本:rmcp-v0.11.0 开始,官方移除了sse协议的支持([breaking] remove SSE transport support (#562)),我们当前的版本是 rmcp-v0.10.0 ,但这样子长久的话,无法跟随官方的最新版本的修复。

所以我想把rmcp库的使用，根据我当前的业务使用，拆分成2个lib库，一个是： mcp-sse 负责sse协议的逻辑处理； 另外一个是 mcp-stream 负责streamable协议的逻辑处理。我们先设计拆分出来这个2个lib库，重构完毕后，我们在看现有的mcp-proxy 模块怎么来代替。

因为我拆分到2个不同的lib库，这样 mcp-sse 模块使用rmcp版本：rmcp-v0.10.0 ,而 mcp-stream 模块使用rmcp版本：rmcp-v0.12.0,这样streamable协议的逻辑，就可以跟随官方的sdk升级了。

另外还有个点，如何去支持 ：stateful_mode: true,我们的逻辑怎么去支持呢？ 