//! OSS域名替换示例
//!
//! 展示如何使用域名替换功能来解决跨域问题

use oss_client::{replace_oss_domain, replace_oss_domains_batch};

fn main() {
    println!("=== OSS域名替换示例 ===\n");

    // 示例1: 单个URL替换
    let original_url = "https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/image.jpg";
    let replaced_url = replace_oss_domain(original_url);

    println!("原始URL: {original_url}");
    println!("替换后: {replaced_url}\n");

    // 示例2: 带路径的URL替换
    let original_url_with_path =
        "https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/folder/subfolder/image.png";
    let replaced_url_with_path = replace_oss_domain(original_url_with_path);

    println!("带路径的原始URL: {original_url_with_path}");
    println!("替换后: {replaced_url_with_path}\n");

    // 示例3: 带查询参数的URL替换
    let original_url_with_query =
        "https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/image.jpg?version=1.0&size=large";
    let replaced_url_with_query = replace_oss_domain(original_url_with_query);

    println!("带查询参数的原始URL: {original_url_with_query}");
    println!("替换后: {replaced_url_with_query}\n");

    // 示例4: 不匹配的域名保持不变
    let other_url = "https://other-domain.com/image.jpg";
    let unchanged_url = replace_oss_domain(other_url);

    println!("其他域名URL: {other_url}");
    println!("保持不变: {unchanged_url}\n");

    // 示例5: 批量替换
    let urls = vec![
        "https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/image1.jpg".to_string(),
        "https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/image2.jpg".to_string(),
        "https://other-domain.com/image3.jpg".to_string(),
        "https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/folder/image4.png".to_string(),
    ];

    println!("批量替换前的URLs:");
    for (i, url) in urls.iter().enumerate() {
        println!("  {}: {}", i + 1, url);
    }

    let replaced_urls = replace_oss_domains_batch(&urls);

    println!("\n批量替换后的URLs:");
    for (i, url) in replaced_urls.iter().enumerate() {
        println!("  {}: {}", i + 1, url);
    }

    println!("\n=== 使用场景说明 ===");
    println!("1. 解决跨域问题: 将阿里云OSS域名替换为自定义域名");
    println!("2. 图片预览: 在网页中正常显示OSS存储的图片");
    println!("3. 批量处理: 一次性处理多个URL的域名替换");
    println!("4. 保持兼容: 不匹配的域名保持不变");
}
