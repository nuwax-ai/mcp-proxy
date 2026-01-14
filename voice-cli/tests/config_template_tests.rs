#[cfg(test)]
mod config_template_tests {

    #[test]
    fn test_server_template_inclusion() {
        // 测试server配置模板
        let server_template = include_str!("../templates/server-config.yml.template");

        // 验证配置模板不为空
        assert!(!server_template.is_empty(), "server配置模板不应该为空");

        // 验证模板包含关键配置项
        assert!(server_template.contains("server:"), "应该包含server配置");
        assert!(server_template.contains("whisper:"), "应该包含whisper配置");
        assert!(server_template.contains("logging:"), "应该包含logging配置");
        assert!(server_template.contains("daemon:"), "应该包含daemon配置");

        // Server模板不应该包含cluster配置
        assert!(
            !server_template.contains("cluster:"),
            "server模板不应该包含cluster配置"
        );
        assert!(
            !server_template.contains("load_balancer:"),
            "server模板不应该包含load_balancer配置"
        );

        println!(
            "✅ Server配置模板包含所有必要的配置项，长度: {} 字节",
            server_template.len()
        );
    }

    #[test]
    fn test_server_template_yaml_validity() {
        // 测试server模板内容是否为有效的YAML格式
        let template_content = include_str!("../templates/server-config.yml.template");

        // 尝试解析YAML
        let yaml_result: Result<serde_yaml::Value, _> = serde_yaml::from_str(template_content);

        match yaml_result {
            Ok(yaml_value) => {
                println!("✅ Server配置模板YAML格式有效");

                // 验证关键配置节点存在
                assert!(
                    yaml_value.get("server").is_some(),
                    "server模板应该有server配置节点"
                );
                assert!(
                    yaml_value.get("whisper").is_some(),
                    "server模板应该有whisper配置节点"
                );
                assert!(
                    yaml_value.get("logging").is_some(),
                    "server模板应该有logging配置节点"
                );
                assert!(
                    yaml_value.get("daemon").is_some(),
                    "server模板应该有daemon配置节点"
                );
            }
            Err(e) => {
                panic!("Server配置模板YAML格式无效: {}", e);
            }
        }
    }
}
