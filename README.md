# Backup Helper

App desktop sao lưu file tự động cho **Windows, macOS, Linux**. Chạy ngầm dưới khay
hệ thống, chỉ copy file *có thay đổi* (incremental), theo thời gian thực hoặc định kỳ.
Giao diện tiếng Việt tối giản, xây bằng [Tauri](https://tauri.app) nên rất nhẹ.

## Tính năng

- Chọn nhiều cặp thư mục **nguồn → đích**.
- 2 chế độ: **thời gian thực** (theo dõi thay đổi) hoặc **định kỳ** (mặc định 30 phút, chỉnh được).
- 2 cách xử lý khi file nguồn bị xóa (chọn cho từng cặp):
  - **Giữ lại** bản backup (an toàn) — mặc định.
  - **Mirror**: xóa ở đích theo nguồn.
- Chỉ copy file thay đổi (so sánh kích thước + thời gian sửa) → nhẹ, nhanh.
- **Tự khởi động** cùng máy, thu nhỏ xuống khay hệ thống.
- **Nhật ký chi tiết 2 ngày gần nhất** phục vụ audit (tự xóa log cũ hơn).

## Chạy khi phát triển

```bash
cd ~/Rich/backup-helper
npm install
npm run dev
```

## Đóng gói bản cài đặt

```bash
npm run build
```

Sản phẩm nằm trong `src-tauri/target/release/bundle/`:
- Windows: `.msi` / `.exe`
- macOS: `.dmg` / `.app`
- Linux: `.AppImage` / `.deb`

> Mỗi bản cài chỉ đóng gói được cho HĐH đang build. Muốn đủ 3 nền tảng thì build
> trên từng HĐH (hoặc dùng CI).

## Vị trí dữ liệu

- Cấu hình: thư mục config của app theo HĐH (ví dụ macOS: `~/Library/Application Support/com.backuphelper.app/config.json`).
- Log: thư mục log của app (`backup-YYYY-MM-DD.log`), chỉ giữ 2 ngày gần nhất.
