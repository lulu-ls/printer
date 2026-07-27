// 手写轻量 PDF 生成器（无第三方依赖）
// 用途：把图片 / 文本统一转换为 PDF，供系统打印 API 发送。
// 说明：
//  - 图片：直接把 JPEG 字节以 DCTDecode 嵌入（不解码，零质量损失）。
//  - 文本：使用内置 Helvetica 字体，仅支持 Latin-1；含中文等非 Latin-1
//    字符时应改用系统原生文本打印（见 converter/text.rs）。

// 部分函数仅在 macOS 路径下使用，Windows 构建会报 dead_code；这里仅在该平台抑制。
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use std::io::Write;

/// `str::ceil_char_boundary` 在稳定版 Rust 中尚不稳定，这里提供等价实现：
/// 返回 >= `index` 的最小字符边界索引（即不会把多字节字符从中间截断）。
fn ceil_char_boundary(s: &str, mut index: usize) -> usize {
    if index >= s.len() {
        return s.len();
    }
    while !s.is_char_boundary(index) {
        index += 1;
    }
    index
}

/// 从 JPEG 字节中解析宽高与颜色分量数（解析 SOF 标记）。
fn jpeg_info(jpeg: &[u8]) -> Option<(u32, u32, u8)> {
    if jpeg.len() < 4 || &jpeg[0..2] != b"\xff\xd8" {
        return None;
    }
    let mut i = 2;
    while i < jpeg.len() - 9 {
        if jpeg[i] != 0xff {
            i += 1;
            continue;
        }
        let marker = jpeg[i + 1];
        // SOF markers: 0xC0-0xCF except 0xC4,0xC8,0xCC
        if (0xc0..=0xcf).contains(&marker) && marker != 0xc4 && marker != 0xc8 && marker != 0xcc {
            let height = u16::from_be_bytes([jpeg[i + 5], jpeg[i + 6]]) as u32;
            let width = u16::from_be_bytes([jpeg[i + 7], jpeg[i + 8]]) as u32;
            let components = jpeg[i + 9];
            return Some((width, height, components));
        }
        let seg_len = u16::from_be_bytes([jpeg[i + 2], jpeg[i + 3]]) as usize;
        i += seg_len + 2;
    }
    None
}

/// 把一张 JPEG 图片包装为单页 PDF，自动缩放适配 A4 页面。
pub fn jpeg_to_pdf(jpeg: &[u8]) -> Result<Vec<u8>, String> {
    let (w, h, components) = jpeg_info(jpeg).ok_or("无法解析 JPEG 尺寸")?;
    let (pw, ph): (f64, f64) = if w >= h { (842.0, 595.0) } else { (595.0, 842.0) };
    let scale = (pw / w as f64).min(ph / h as f64);
    let dw = (w as f64 * scale).round();
    let dh = (h as f64 * scale).round();

    // 颜色空间：按 JPEG 分量数选择（1=灰度，3=RGB）。
    let colorspace = match components {
        1 => "/DeviceGray",
        4 => "/DeviceCMYK",
        _ => "/DeviceRGB",
    };

    let content = format!("q {:.2} 0 0 {:.2} 0 0 cm /Im0 Do Q", dw, dh);

    let mut buf: Vec<u8> = Vec::new();
    let _ = buf.write_all(b"%PDF-1.4\n");

    let mut offsets: Vec<usize> = Vec::new();
    let write_obj = |buf: &mut Vec<u8>, offsets: &mut Vec<usize>, body: &[u8]| {
        offsets.push(buf.len());
        // offsets 为 0 索引：第 i 个元素对应对象 i+1，故对象编号 = offsets.len()
        let _ = buf.write_fmt(format_args!("{} 0 obj\n", offsets.len()));
        let _ = buf.write_all(body);
        let _ = buf.write_all(b"\nendobj\n");
    };

    // 1 Catalog
    write_obj(&mut buf, &mut offsets, b"<< /Type /Catalog /Pages 2 0 R >>");
    // 2 Pages
    write_obj(
        &mut buf,
        &mut offsets,
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
    );
    // 3 Page
    let page = format!(
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {:.2} {:.2}] \
         /Resources << /XObject << /Im0 4 0 R >> >> /Contents 5 0 R >>",
        dw, dh
    );
    write_obj(&mut buf, &mut offsets, page.as_bytes());
    // 4 Image XObject
    let img = format!(
        "<< /Type /XObject /Subtype /Image /Width {} /Height {} \
         /ColorSpace {} /BitsPerComponent 8 /Filter /DCTDecode /Length {} >>",
        w, h, colorspace, jpeg.len()
    );
    offsets.push(buf.len());
    let _ = buf.write_fmt(format_args!("4 0 obj\n"));
    let _ = buf.write_all(&img.as_bytes());
    let _ = buf.write_all(b"\nstream\n");
    let _ = buf.write_all(jpeg);
    let _ = buf.write_all(b"\nendstream\nendobj\n");
    // 5 Contents
    let content_obj = format!("<< /Length {} >>", content.len());
    offsets.push(buf.len());
    let _ = buf.write_fmt(format_args!("5 0 obj\n"));
    let _ = buf.write_all(content_obj.as_bytes());
    let _ = buf.write_all(b"\nstream\n");
    let _ = buf.write_all(content.as_bytes());
    let _ = buf.write_all(b"\nendstream\nendobj\n");

    // xref
    let xref_offset = buf.len();
    let _ = buf.write_fmt(format_args!("xref\n0 {}\n", offsets.len() + 1));
    let _ = buf.write_all(b"0000000000 65535 f \n");
    for off in &offsets {
        let _ = buf.write_fmt(format_args!("{:010} 00000 n \n", off));
    }
    let _ = buf.write_fmt(format_args!(
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF\n",
        offsets.len() + 1,
        xref_offset
    ));

    Ok(buf)
}

/// 把纯文本包装为多页 PDF（Helvetica 12pt，Latin-1 字符）。
/// 遇到非 Latin-1 字符（如中文）返回 Err，调用方应改用系统文本打印。
pub fn text_to_pdf(text: &str) -> Result<Vec<u8>, String> {
    let page_w = 595.0;
    let page_h = 842.0;
    let margin = 50.0;
    let font_size = 12.0;
    let leading = 16.0;
    let max_chars = (((page_w - 2.0 * margin) / (font_size * 0.5)) as f64).floor() as usize;
    let max_lines = (((page_h - 2.0 * margin) / leading) as f64).floor() as usize;

    let mut cur_lines: Vec<String> = Vec::new();

    for raw_line in text.lines() {
        // Latin-1 校验
        for ch in raw_line.chars() {
            if ch as u32 > 0xff {
                return Err("文本包含非 Latin-1 字符，请使用系统文本打印".into());
            }
        }
        // 超长行折行
        let mut line = raw_line;
        while line.len() > max_chars {
            let split = ceil_char_boundary(line, max_chars.min(line.len()));
            cur_lines.push(line[..split].to_string());
            line = &line[split..];
        }
        cur_lines.push(line.to_string());
    }

    let mut pages: Vec<String> = Vec::new();
    for chunk in cur_lines.chunks(max_lines) {
        let mut content = String::new();
        content.push_str(&format!("BT /F1 {} Tf {} TL\n", font_size, leading));
        content.push_str(&format!("{} {} Td\n", margin, page_h - margin));
        for (i, l) in chunk.iter().enumerate() {
            if i > 0 {
                content.push_str("T*\n");
            }
            let escaped = l.replace('\\', "\\\\").replace('(', "\\(").replace(')', "\\)");
            content.push_str(&format!("({}) Tj\n", escaped));
        }
        content.push_str("ET");
        pages.push(content);
    }

    write_text_doc(pages, page_w, page_h)
}

// 文本 PDF 构建：对象编号固定，避免错乱。
// 1: Catalog  2: Pages  3,5,7..: Page  4,6,8..: Content  font: 3+2N
fn write_text_doc(pages: Vec<String>, page_w: f64, page_h: f64) -> Result<Vec<u8>, String> {
    let font_obj_num = 3 + pages.len() * 2;

    let mut objs: Vec<(usize, String)> = Vec::new();
    objs.push((1, "<< /Type /Catalog /Pages 2 0 R >>".to_string()));

    let mut kids = String::new();
    for p in 0..pages.len() {
        kids.push_str(&format!("{} 0 R ", 3 + p * 2));
    }
    objs.push((
        2,
        format!(
            "<< /Type /Pages /Kids [{}] /Count {} >>",
            kids.trim(),
            pages.len()
        ),
    ));

    for (p, content) in pages.iter().enumerate() {
        let page_num = 3 + p * 2;
        let content_num = page_num + 1;
        objs.push((
            page_num,
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {:.0} {:.0}] \
                 /Resources << /Font << /F1 {} 0 R >> >> /Contents {} 0 R >>",
                page_w, page_h, font_obj_num, content_num
            ),
        ));
        objs.push((
            content_num,
            format!("<< /Length {} >>\nstream\n{}\nendstream", content.len(), content),
        ));
    }
    objs.push((
        font_obj_num,
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"
            .to_string(),
    ));

    objs.sort_by_key(|(n, _)| *n);

    let mut buf: Vec<u8> = Vec::new();
    let _ = buf.write_all(b"%PDF-1.4\n");
    let mut offsets: Vec<usize> = vec![0; objs.len() + 1];
    for (n, body) in &objs {
        offsets[*n] = buf.len();
        let _ = buf.write_fmt(format_args!("{} 0 obj\n{}\nendobj\n", n, body));
    }

    let xref_offset = buf.len();
    let _ = buf.write_fmt(format_args!("xref\n0 {}\n", objs.len() + 1));
    let _ = buf.write_all(b"0000000000 65535 f \n");
    for n in 1..=objs.len() {
        let _ = buf.write_fmt(format_args!("{:010} 00000 n \n", offsets[n]));
    }
    let _ = buf.write_fmt(format_args!(
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF\n",
        objs.len() + 1,
        xref_offset
    ));

    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 校验生成的 PDF 结构：头部、尾部、xref 偏移是否指向正确的对象。
    /// 直接基于字节操作，避免二进制内容破坏 UTF-8 偏移。
    fn validate_pdf(bytes: &[u8]) {
        assert!(bytes.starts_with(b"%PDF"), "缺少 PDF 头");
        assert!(
            bytes.ends_with(b"%%EOF\n") || bytes.ends_with(b"%%EOF"),
            "缺少 PDF 尾"
        );

        // 找到 startxref，解析其后紧跟的偏移数字
        let sx = find_subseq(bytes, b"startxref").expect("缺少 startxref");
        let after_sx = &bytes[sx + b"startxref".len()..];
        let xref_off = parse_first_usize(after_sx).expect("startxref 偏移非法");
        assert!(
            bytes[xref_off..].starts_with(b"xref"),
            "xref 偏移错误"
        );

        // xref\n0 N\n 之后每行一个条目 "<10位偏移> <5位代> f|n"
        let mut p = xref_off + b"xref".len();
        p = skip_line(bytes, p);
        let n = parse_first_usize(&bytes[p..]).expect("xref 计数非法");
        p = skip_line(bytes, p);
        let mut entries: Vec<usize> = Vec::new();
        for _ in 0..n {
            let off = parse_first_usize(&bytes[p..]).expect("xref 条目偏移非法");
            entries.push(off);
            p = skip_line(bytes, p);
        }
        for obj_num in 1..n {
            let off = entries[obj_num];
            let prefix = format!("{} 0 obj", obj_num).into_bytes();
            assert!(
                bytes[off..].starts_with(&prefix),
                "对象 {} 偏移指向错误内容",
                obj_num
            );
        }
    }

    fn find_subseq(hay: &[u8], needle: &[u8]) -> Option<usize> {
        hay.windows(needle.len()).position(|w| w == needle)
    }

    /// 跳过当前行（含换行符）。
    fn skip_line(bytes: &[u8], mut p: usize) -> usize {
        while p < bytes.len() && bytes[p] != b'\n' {
            p += 1;
        }
        if p < bytes.len() {
            p += 1; // 越过 '\n'
        }
        p
    }

    /// 从字节序列中解析第一个十进制无符号整数。
    fn parse_first_usize(bytes: &[u8]) -> Option<usize> {
        let mut p = 0;
        while p < bytes.len() && !bytes[p].is_ascii_digit() {
            p += 1;
        }
        let start = p;
        while p < bytes.len() && bytes[p].is_ascii_digit() {
            p += 1;
        }
        if start == p {
            return None;
        }
        std::str::from_utf8(&bytes[start..p]).ok()?.parse().ok()
    }

    #[test]
    fn text_pdf_valid() {
        let pdf = text_to_pdf("Hello world\nSecond line\nRust print pipeline").unwrap();
        validate_pdf(&pdf);
    }

    #[test]
    fn text_pdf_chinese_falls_back() {
        assert!(text_to_pdf("中文内容测试").is_err());
    }

    #[test]
    fn image_pdf_valid() {
        // 用系统 sips 造一个真实 JPEG，验证 DCTDecode 嵌入后的 PDF 结构
        let jpg = std::env::temp_dir().join("pdf_test_img.jpg");
        let status = std::process::Command::new("sips")
            .args(["-s", "format", "jpeg", "--out"])
            .arg(&jpg)
            .arg("/Users/liuxs/github/printer/app-icon.png")
            .status()
            .expect("sips 不可用");
        assert!(status.success(), "sips 转换失败");
        let bytes = std::fs::read(&jpg).expect("读取 JPEG 失败");
        let pdf = jpeg_to_pdf(&bytes).expect("生成图片 PDF 失败");
        validate_pdf(&pdf);
        let text = String::from_utf8_lossy(&pdf);
        assert!(text.contains("/DCTDecode"), "缺少 DCTDecode 图像流");
    }
}
