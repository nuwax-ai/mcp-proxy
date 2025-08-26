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
        assert!(!server_template.contains("cluster:"), "server模板不应该包含cluster配置");
        assert!(!server_template.contains("load_balancer:"), "server模板不应该包含load_balancer配置");

        println!(
            "✅ Server配置模板包含所有必要的配置项，长度: {} 字节",
            server_template.len()
        );
    }

    #[test]
    fn test_cluster_template_inclusion() {
        // 测试cluster配置模板
        let cluster_template = include_str!("../templates/cluster-config.yml.template");

        // 验证配置模板不为空
        assert!(!cluster_template.is_empty(), "cluster配置模板不应该为空");

        // 验证模板包含关键配置项
        assert!(cluster_template.contains("server:"), "应该包含server配置");
        assert!(cluster_template.contains("whisper:"), "应该包含whisper配置");
        assert!(cluster_template.contains("cluster:"), "应该包含cluster配置");
        assert!(cluster_template.contains("logging:"), "应该包含logging配置");
        assert!(cluster_template.contains("daemon:"), "应该包含daemon配置");
        assert!(
            cluster_template.contains("leader_can_process_tasks:"),
            "应该包含leader_can_process_tasks配置"
        );
        assert!(
            cluster_template.contains("grpc_port:"),
            "应该包含grpc_port配置"
        );

        // Cluster模板不应该包含load_balancer配置
        assert!(!cluster_template.contains("load_balancer:"), "cluster模板不应该包含load_balancer配置");

        println!(
            "✅ Cluster配置模板包含所有必要的配置项，长度: {} 字节",
            cluster_template.len()
        );
    }

    #[test]
    fn test_lb_template_inclusion() {
        // 测试load balancer配置模板
        let lb_template = include_str!("../templates/lb-config.yml.template");

        // 验证配置模板不为空
        assert!(!lb_template.is_empty(), "lb配置模板不应该为空");

        // 验证模板包含关键配置项
        assert!(lb_template.contains("load_balancer:"), "应该包含load_balancer配置");
        assert!(lb_template.contains("logging:"), "应该包含logging配置");
        assert!(lb_template.contains("daemon:"), "应该包含daemon配置");
        assert!(
            lb_template.contains("health_check_interval:"),
            "应该包含health_check_interval配置"
        );

        // LB模板不应该包含server/whisper/cluster配置
        assert!(!lb_template.contains("server:"), "lb模板不应该包含server配置");
        assert!(!lb_template.contains("whisper:"), "lb模板不应该包含whisper配置");
        assert!(!lb_template.contains("cluster:"), "lb模板不应该包含cluster配置");

        println!(
            "✅ Load Balancer配置模板包含所有必要的配置项，长度: {} 字节",
            lb_template.len()
        );
    }

    #[test]
    fn test_all_templates_yaml_validity() {
        // 测试所有模板内容是否为有效的YAML格式
        let templates = [
            ("server", include_str!("../templates/server-config.yml.template")),
            ("cluster", include_str!("../templates/cluster-config.yml.template")),
            ("load_balancer", include_str!("../templates/lb-config.yml.template")),
        ];

        for (template_name, template_content) in templates {
            // 尝试解析YAML
            let yaml_result: Result<serde_yaml::Value, _> = serde_yaml::from_str(template_content);

            match yaml_result {
                Ok(yaml_value) => {
                    println!("✅ {} 配置模板YAML格式有效", template_name);

                    // 验证关键配置节点存在
                    match template_name {
                        "server" => {
                            assert!(yaml_value.get("server").is_some(), "server模板应该有server配置节点");
                            assert!(yaml_value.get("whisper").is_some(), "server模板应该有whisper配置节点");
                            assert!(yaml_value.get("logging").is_some(), "server模板应该有logging配置节点");
                            assert!(yaml_value.get("daemon").is_some(), "server模板应该有daemon配置节点");
                        }
                        "cluster" => {
                            assert!(yaml_value.get("server").is_some(), "cluster模板应该有server配置节点");
                            assert!(yaml_value.get("whisper").is_some(), "cluster模板应该有whisper配置节点");
                            assert!(yaml_value.get("cluster").is_some(), "cluster模板应该有cluster配置节点");
                            assert!(yaml_value.get("logging").is_some(), "cluster模板应该有logging配置节点");
                            assert!(yaml_value.get("daemon").is_some(), "cluster模板应该有daemon配置节点");
                        }
                        "load_balancer" => {
                            assert!(yaml_value.get("load_balancer").is_some(), "lb模板应该有load_balancer配置节点");
                            assert!(yaml_value.get("logging").is_some(), "lb模板应该有logging配置节点");
                            assert!(yaml_value.get("daemon").is_some(), "lb模板应该有daemon配置节点");
                        }
                        _ => panic!("未知的模板类型: {}", template_name),
                    }
                }
                Err(e) => {
                    panic!("{} 配置模板YAML格式无效: {}", template_name, e);
                }
            }
        }
    }
}
