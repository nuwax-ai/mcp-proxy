CREATE TABLE IF NOT EXISTS mcp_plugin (
    id              bigint          NOT NULL AUTO_INCREMENT PRIMARY KEY COMMENT '主键',
    name            varchar(128)    NOT NULL COMMENT '插件名称',
    command         varchar(512)    NOT NULL COMMENT '启动命令,用户输入,或者从json配置中解析获取',
    args            JSON            NULL COMMENT '启动参数(JSON格式),例如:[{"name":"port","value":"8000"}]',
    envs            JSON            NULL COMMENT '环境变量(JSON格式),例如:[{"name":"PORT","value":"8000"}]',
    mounts          JSON            NULL COMMENT '挂载目录(JSON格式),例如:[{"name":"/data/mcp","path":"/data/mcp"}]',
    port            int             NULL COMMENT '端口,例如:8000',
    external_url    varchar(512)    NULL COMMENT '外部访问路径,例如:http://192.168.1.1:8000',
    container_name  varchar(128)    NULL COMMENT '容器名称,例如:mcp-plugin',
    sse_path        varchar(128)    NULL COMMENT 'SSE路径,例如:/sse,注意不要与其他 Server 重复',
    config_json     JSON            NULL COMMENT '插件配置(JSON)',
    status          tinyint         DEFAULT 2 NOT NULL COMMENT 'MCP启动状态:1:RUNNING(运行中),2:STOPPED(已停止),3:ERROR(错误)',
    enabled         tinyint         DEFAULT 1 NOT NULL COMMENT '是否启用:1(启用),0(禁用)',
    -- 公共字段
    created         datetime        DEFAULT CURRENT_TIMESTAMP NOT NULL COMMENT '创建时间',
    creator_id      bigint                                  NULL COMMENT '创建人id',
    creator_name    varchar(64)                             NULL COMMENT '创建人',
    modified        datetime        DEFAULT CURRENT_TIMESTAMP NULL ON UPDATE CURRENT_TIMESTAMP COMMENT '更新时间',
    modified_id     bigint                                  NULL COMMENT '最后修改人id',
    modified_name   varchar(64)                             NULL COMMENT '最后修改人',
    yn              tinyint         DEFAULT 1               NULL COMMENT '逻辑标记,1:有效;-1:无效'
) DEFAULT CHARSET=utf8mb4 COMMENT='MCP插件服务表';