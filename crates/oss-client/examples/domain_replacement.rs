//! OSS域名替换示例
//!
//! 展示如何使用域名替换功能来解决跨域问题

use oss_client::{replace_oss_domain, replace_oss_domains_batch};

fn main() {
    println!("=== OSS domain name replacement example ===\\n");

    // 示例1: 单个URL替换
    let original_url = "https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/image.jpg";
    let replaced_url = replace_oss_domain(original_url);

    println!("Original URL: {original_url}");
    println!("After replacement: {replaced_url}\\n");

    // 示例2: 带路径的URL替换
    let original_url_with_path =
        "https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/folder/subfolder/image.png";
    let replaced_url_with_path = replace_oss_domain(original_url_with_path);

    println!("Original URL with path: {original_url_with_path}");
    println!("After replacement: {replaced_url_with_path}\\n");

    // 示例3: 带查询参数的URL替换
    let original_url_with_query =
        "https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/image.jpg?version=1.0&size=large";
    let replaced_url_with_query = replace_oss_domain(original_url_with_query);

    println!("Original URL with query parameters: {original_url_with_query}");
    println!("After replacement: {replaced_url_with_query}\\n");

    // 示例4: 不匹配的域名保持不变
    let other_url = "https://other-domain.com/image.jpg";
    let unchanged_url = replace_oss_domain(other_url);

    println!("Other domain name URL: {other_url}");
    println!("Remain unchanged: {unchanged_url}\\n");

    // 示例5: 批量替换
    let urls = vec![
        "https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/image1.jpg".to_string(),
        "https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/image2.jpg".to_string(),
        "https://other-domain.com/image3.jpg".to_string(),
        "https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/folder/image4.png".to_string(),
    ];

    println!("URLs before batch replacement:");
    for (i, url) in urls.iter().enumerate() {
        println!("  {}: {}", i + 1, url);
    }

    let replaced_urls = replace_oss_domains_batch(&urls);

    println!("\\nURLs after batch replacement:");
    for (i, url) in replaced_urls.iter().enumerate() {
        println!("  {}: {}", i + 1, url);
    }

    println!("\\n=== Usage scenario description ===");
    println!(
        "1. Solve the cross-domain problem: Replace the Alibaba Cloud OSS domain name with a custom domain name"
    );
    println!("2. Image preview: Normally display images stored in OSS on the web page");
    println!(
        "3. Batch processing: Process the domain name replacement of multiple URLs at one time"
    );
    println!("4. Maintain compatibility: Unmatched domain names remain unchanged");
}
