use anyhow::{Context, Result};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use tracing::{debug, info};

/// IP地址类型，用于分类和优先级排序
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum IpType {
    /// 公网IP地址（最优先）
    Public = 0,
    /// 私有网络IP地址（次优先）
    Private = 1,
    /// 本地回环地址（最低优先级）
    Loopback = 2,
}

/// 网络接口信息
#[derive(Debug, Clone)]
pub struct NetworkInterface {
    pub name: String,
    pub ip: IpAddr,
    pub ip_type: IpType,
    pub is_up: bool,
}

/// IP发现配置
#[derive(Debug, Clone)]
pub struct IpDiscoveryConfig {
    /// 是否偏好IPv4地址
    pub prefer_ipv4: bool,
    /// 是否包含回环地址
    pub include_loopback: bool,
    /// 优先使用的网络接口名称模式
    pub preferred_interface_patterns: Vec<String>,
    /// 排除的网络接口名称模式
    pub excluded_interface_patterns: Vec<String>,
}

impl Default for IpDiscoveryConfig {
    fn default() -> Self {
        Self {
            prefer_ipv4: true,
            include_loopback: false,
            preferred_interface_patterns: vec![
                "eth".to_string(),
                "en".to_string(),   // macOS以太网
                "wlan".to_string(), // Linux无线网
                "wl".to_string(),   // 无线网络
            ],
            excluded_interface_patterns: vec![
                "docker".to_string(),
                "br-".to_string(),   // Docker桥接网络
                "veth".to_string(),  // Docker虚拟以太网
                "lo".to_string(),    // 回环接口
                "virbr".to_string(), // libvirt桥接网络
            ],
        }
    }
}

/// IP地址发现工具
pub struct IpDiscovery {
    config: IpDiscoveryConfig,
}

impl IpDiscovery {
    /// 创建新的IP发现实例
    pub fn new(config: IpDiscoveryConfig) -> Self {
        Self { config }
    }

    /// 使用默认配置创建IP发现实例
    pub fn default() -> Self {
        Self::new(IpDiscoveryConfig::default())
    }

    /// 获取最佳的本地IP地址
    pub fn get_best_local_ip(&self) -> Result<IpAddr> {
        let interfaces = self.list_network_interfaces()?;

        if interfaces.is_empty() {
            anyhow::bail!("No network interfaces found");
        }

        // 按优先级排序并选择最佳IP
        let best_ip = self.select_best_ip(&interfaces)?;

        info!(
            "Selected best local IP: {} (from interface: {})",
            best_ip.ip, best_ip.name
        );

        Ok(best_ip.ip)
    }

    /// 获取推荐的集群通告地址
    pub fn get_cluster_advertise_ip(&self) -> Result<IpAddr> {
        // 对于集群，我们通常不想使用回环地址
        let mut config = self.config.clone();
        config.include_loopback = false;

        let discovery = IpDiscovery::new(config);
        discovery.get_best_local_ip()
    }

    /// 列出所有网络接口
    pub fn list_network_interfaces(&self) -> Result<Vec<NetworkInterface>> {
        let mut interfaces = Vec::new();

        // 使用if-addrs库获取网络接口信息
        let addrs = if_addrs::get_if_addrs().context("Failed to get network interfaces")?;

        for addr in addrs {
            let ip = addr.ip();
            let name = addr.name.to_string();
            let is_up = !addr.is_loopback();

            // 过滤掉排除的接口
            if self.is_interface_excluded(&name) {
                debug!("Excluding interface: {} ({})", name, ip);
                continue;
            }

            // 根据配置过滤回环地址
            if ip.is_loopback() && !self.config.include_loopback {
                debug!("Excluding loopback interface: {} ({})", name, ip);
                continue;
            }

            let ip_type = self.classify_ip(&ip);

            interfaces.push(NetworkInterface {
                name,
                ip,
                ip_type,
                is_up,
            });
        }

        debug!("Found {} network interfaces", interfaces.len());
        for interface in &interfaces {
            debug!(
                "Interface: {} -> {} ({:?})",
                interface.name, interface.ip, interface.ip_type
            );
        }

        Ok(interfaces)
    }

    /// 选择最佳的IP地址
    fn select_best_ip<'a>(
        &self,
        interfaces: &'a [NetworkInterface],
    ) -> Result<&'a NetworkInterface> {
        if interfaces.is_empty() {
            anyhow::bail!("No suitable network interfaces available");
        }

        // 创建评分系统
        let mut scored_interfaces = Vec::new();

        for interface in interfaces {
            let score = self.calculate_interface_score(interface);
            scored_interfaces.push((score, interface));
        }

        // 按分数排序（分数越低越好）
        scored_interfaces.sort_by_key(|(score, _)| *score);

        let best = scored_interfaces[0].1;
        debug!(
            "Selected interface: {} (score: {})",
            best.name, scored_interfaces[0].0
        );

        Ok(best)
    }

    /// 计算接口得分（分数越低越好）
    fn calculate_interface_score(&self, interface: &NetworkInterface) -> u32 {
        let mut score = 0u32;

        // IP类型得分（公网 < 私有 < 回环）
        score += (interface.ip_type as u32) * 1000;

        // IPv4 vs IPv6偏好
        if self.config.prefer_ipv4 {
            match interface.ip {
                IpAddr::V4(_) => score += 0,
                IpAddr::V6(_) => score += 100,
            }
        } else {
            match interface.ip {
                IpAddr::V4(_) => score += 100,
                IpAddr::V6(_) => score += 0,
            }
        }

        // 接口状态得分
        if !interface.is_up {
            score += 500;
        }

        // 优先接口名称匹配
        let interface_priority = self.get_interface_name_priority(&interface.name);
        score += interface_priority * 10;

        score
    }

    /// 获取接口名称优先级（数字越小优先级越高）
    fn get_interface_name_priority(&self, name: &str) -> u32 {
        // 检查是否匹配优先模式
        for (i, pattern) in self.config.preferred_interface_patterns.iter().enumerate() {
            if name.starts_with(pattern) {
                return i as u32;
            }
        }

        // 如果没有匹配优先模式，返回较高的优先级数字
        1000
    }

    /// 检查接口是否应该被排除
    fn is_interface_excluded(&self, name: &str) -> bool {
        for pattern in &self.config.excluded_interface_patterns {
            if name.starts_with(pattern) {
                return true;
            }
        }
        false
    }

    /// 分类IP地址类型
    fn classify_ip(&self, ip: &IpAddr) -> IpType {
        match ip {
            IpAddr::V4(v4) => self.classify_ipv4(v4),
            IpAddr::V6(v6) => self.classify_ipv6(v6),
        }
    }

    /// 分类IPv4地址
    fn classify_ipv4(&self, ip: &Ipv4Addr) -> IpType {
        if ip.is_loopback() {
            IpType::Loopback
        } else if ip.is_private() {
            IpType::Private
        } else {
            IpType::Public
        }
    }

    /// 分类IPv6地址
    fn classify_ipv6(&self, ip: &Ipv6Addr) -> IpType {
        if ip.is_loopback() {
            IpType::Loopback
        } else if self.is_ipv6_private(ip) {
            IpType::Private
        } else {
            IpType::Public
        }
    }

    /// 检查IPv6地址是否为私有地址
    fn is_ipv6_private(&self, ip: &Ipv6Addr) -> bool {
        // IPv6私有地址范围
        let segments = ip.segments();

        // 本地链路地址 (fe80::/10)
        if segments[0] & 0xffc0 == 0xfe80 {
            return true;
        }

        // 唯一本地地址 (fc00::/7)
        if segments[0] & 0xfe00 == 0xfc00 {
            return true;
        }

        false
    }

    /// 验证IP地址是否可达
    pub async fn validate_ip_reachability(&self, ip: &IpAddr, port: u16) -> Result<bool> {
        use tokio::net::TcpStream;
        use tokio::time::{timeout, Duration};

        let addr = format!("{}:{}", ip, port);

        match timeout(Duration::from_secs(3), TcpStream::connect(&addr)).await {
            Ok(Ok(_)) => {
                debug!("IP {} is reachable on port {}", ip, port);
                Ok(true)
            }
            Ok(Err(e)) => {
                debug!("IP {} is not reachable on port {}: {}", ip, port, e);
                Ok(false)
            }
            Err(_) => {
                debug!("Timeout checking reachability of {} on port {}", ip, port);
                Ok(false)
            }
        }
    }

    /// 获取所有可用的IP地址（按优先级排序）
    pub fn get_all_ips(&self) -> Result<Vec<IpAddr>> {
        let interfaces = self.list_network_interfaces()?;
        let mut scored_interfaces = Vec::new();

        for interface in &interfaces {
            let score = self.calculate_interface_score(interface);
            scored_interfaces.push((score, interface.ip));
        }

        scored_interfaces.sort_by_key(|(score, _)| *score);
        Ok(scored_interfaces.into_iter().map(|(_, ip)| ip).collect())
    }
}

/// 便捷函数：获取最佳本地IP地址
pub fn get_local_ip() -> Result<IpAddr> {
    let discovery = IpDiscovery::default();
    discovery.get_best_local_ip()
}

/// 便捷函数：获取集群通告IP地址
pub fn get_cluster_ip() -> Result<IpAddr> {
    let discovery = IpDiscovery::default();
    discovery.get_cluster_advertise_ip()
}

/// 便捷函数：使用local-ip-address库作为备选方案
pub fn get_local_ip_fallback() -> Result<IpAddr> {
    local_ip_address::local_ip().context("Failed to get local IP using fallback method")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ip_classification() {
        let discovery = IpDiscovery::default();

        // 测试IPv4分类
        assert_eq!(
            discovery.classify_ipv4(&"127.0.0.1".parse().unwrap()),
            IpType::Loopback
        );
        assert_eq!(
            discovery.classify_ipv4(&"192.168.1.1".parse().unwrap()),
            IpType::Private
        );
        assert_eq!(
            discovery.classify_ipv4(&"8.8.8.8".parse().unwrap()),
            IpType::Public
        );

        // 测试IPv6分类
        assert_eq!(
            discovery.classify_ipv6(&"::1".parse().unwrap()),
            IpType::Loopback
        );
        assert_eq!(
            discovery.classify_ipv6(&"fe80::1".parse().unwrap()),
            IpType::Private
        );
    }

    #[test]
    fn test_interface_exclusion() {
        let discovery = IpDiscovery::default();

        assert!(discovery.is_interface_excluded("docker0"));
        assert!(discovery.is_interface_excluded("br-1234"));
        assert!(discovery.is_interface_excluded("veth123"));
        assert!(!discovery.is_interface_excluded("eth0"));
        assert!(!discovery.is_interface_excluded("en0"));
    }

    #[tokio::test]
    async fn test_get_local_ip() {
        // 这个测试可能在CI环境中失败，所以只记录结果
        match get_local_ip() {
            Ok(ip) => println!("Local IP: {}", ip),
            Err(e) => println!("Failed to get local IP: {}", e),
        }
    }
}
