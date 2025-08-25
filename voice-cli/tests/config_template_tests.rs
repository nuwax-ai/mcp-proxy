#[cfg(test)]
mod config_template_tests {
    use std::path::PathBuf;
    use tempfile::tempdir;

    #[test]
    fn test_config_template_inclusion() {
        // 测试include_str!宏是否能正确读取配置模板
        let config_template = include_str!("../templates/config.yml.template");

        // 验证配置模板不为空
        assert!(!config_template.is_empty(), "配置模板不应该为空");

        // 验证模板包含关键配置项
        assert!(config_template.contains("server:"), "应该包含server配置");
        assert!(config_template.contains("whisper:"), "应该包含whisper配置");
        assert!(config_template.contains("cluster:"), "应该包含cluster配置");
        assert!(
            config_template.contains("load_balancer:"),
            "应该包含load_balancer配置"
        );
        assert!(
            config_template.contains("leader_can_process_tasks:"),
            "应该包含leader_can_process_tasks配置"
        );
        assert!(
            config_template.contains("grpc_port:"),
            "应该包含grpc_port配置"
        );
        assert!(
            config_template.contains("health_check_interval:"),
            "应该包含health_check_interval配置"
        );

        println!(
            "✅ 配置模板包含所有必要的配置项，长度: {} 字节",
            config_template.len()
        );
    }

    #[test]
    fn test_generate_config_with_template() {
        // 测试使用模板生成配置文件的功能
        let temp_dir = tempdir().expect("创建临时目录失败");
        let config_path = temp_dir.path().join("test_config.yml");

        // 使用我们修改后的方法生成配置
        voice_cli::config::ConfigManager::generate_default_config_with_comments(&config_path)
            .expect("生成配置文件失败");

        // 验证文件已创建
        assert!(config_path.exists(), "配置文件应该已创建");

        // 读取生成的配置文件内容
        let generated_content =
            std::fs::read_to_string(&config_path).expect("读取生成的配置文件失败");

        // 验证生成的内容包含关键配置项
        assert!(
            generated_content.contains("cluster:"),
            "生成的配置应该包含cluster配置"
        );
        assert!(
            generated_content.contains("load_balancer:"),
            "生成的配置应该包含load_balancer配置"
        );
        assert!(
            generated_content.contains("leader_can_process_tasks: true"),
            "生成的配置应该包含leader_can_process_tasks配置"
        );

        println!(
            "✅ 配置文件生成成功，内容长度: {} 字节",
            generated_content.len()
        );
    }

    #[test]
    fn test_template_yaml_validity() {
        // 测试模板内容是否为有效的YAML格式
        let config_template = include_str!("../templates/config.yml.template");

        // 尝试解析YAML
        let yaml_result: Result<serde_yaml::Value, _> = serde_yaml::from_str(config_template);

        match yaml_result {
            Ok(yaml_value) => {
                println!("✅ 配置模板YAML格式有效");

                // 验证关键配置节点存在
                assert!(yaml_value.get("server").is_some(), "应该有server配置节点");
                assert!(yaml_value.get("whisper").is_some(), "应该有whisper配置节点");
                assert!(yaml_value.get("cluster").is_some(), "应该有cluster配置节点");
                assert!(
                    yaml_value.get("load_balancer").is_some(),
                    "应该有load_balancer配置节点"
                );
            }
            Err(e) => {
                panic!("配置模板YAML格式无效: {}", e);
            }
        }
    }
}
