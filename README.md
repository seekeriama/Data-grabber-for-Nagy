# FourPointProbe (Rust)

Viết lại chương trình **FourPointProbe_Handheld.exe** (VB.NET, 2023) bằng Rust, giữ nguyên cách đo của bản gốc và thêm chức năng chọn **định dạng file CSV: Default / ERS / Sequoia**.

Chương trình chạy được với hai loại máy, tự dò trên các cổng COM khi khởi động:

| Máy | Cổng | Ghi chú |
|---|---|---|
| Keithley SourceMeter + đầu đo 4 điểm | 57600 baud, 8N1, XON/XOFF | lệnh SCPI, nhận dạng bằng `*IDN?` có chữ KEITHLEY |
| Nagy | 9600 baud, 8N1 | bắt tay `£` + `l0`, máy tự gửi kết quả `R=...` |

## Build trên Windows

1. Cài Rust: https://rustup.rs (chọn mặc định; nếu được hỏi thì cài thêm *Visual Studio Build Tools* với "Desktop development with C++").
2. Mở Command Prompt trong thư mục này rồi chạy:
   ```
   cargo build --release
   ```
3. File chạy: `target\release\FourPointProbe.exe`. Chỉ cần chép file này sang máy đo (không cần cài gì thêm).

## Build tự động trên GitHub (không cần cài Rust)

File `.github/workflows/build-windows.yml` build exe trên máy Windows của GitHub:

1. Tạo một repository trên GitHub (nên để **Private**), đẩy toàn bộ thư mục này lên (gồm cả `.github`, `.cargo`, `Cargo.lock`).
2. Mỗi lần push lên nhánh `main`, GitHub tự chạy kiểm thử rồi build. Có thể chạy tay: tab **Actions** → **Build Windows** → **Run workflow**.
3. Khi lượt chạy có dấu tích xanh, mở lượt chạy đó, kéo xuống mục **Artifacts**, tải `FourPointProbe-windows-x64` (file zip chứa exe + README).
4. Muốn có bản phát hành cố định: `git tag v1.0.0` rồi `git push --tags`. GitHub tạo mục **Releases** kèm file `FourPointProbe-windows-x64.zip`.

Exe được build kèm thư viện C runtime (`.cargo/config.toml`) nên chạy được trên máy chưa cài Visual C++ Redistributable.

Chạy thử không cần máy đo (để đào tạo hoặc kiểm tra giao diện):
```
FourPointProbe.exe --simulate nagy
FourPointProbe.exe --simulate 4pp
```

## Định dạng file CSV (menu **CSV Format**)

Có 3 ô chọn; bắt buộc chọn ít nhất 1, chọn được cả 3. Mỗi lần **Submit**, kết quả được ghi vào **tất cả** các file đã chọn, nội dung giống hệt nhau, chỉ khác tên file:

| Định dạng | Nagy 5 điểm | Nagy 3 điểm | Nagy Continuous | 4-Point Probe |
|---|---|---|---|---|
| Default | `Five-Points-20261017.csv` | `Three-Point-20261017.csv` | `Continuous-20261017.csv` | `20261017.csv` |
| ERS | `ERS_Five-Points-20261017.csv` | `ERS_Three-Point-…` | `ERS_Continuous-…` | `ERS_20261017.csv` |
| Sequoia | `Sequoia_Five-Points-20261017.csv` | `Sequoia_Three-Point-…` | `Sequoia_Continuous-…` | `Sequoia_20261017.csv` |

Ví dụ một dòng Nagy 5 điểm (đúng cột như file mẫu):
```
ProductLine,Measurement time,RunNumber,RunMinute,Offset,LEMeasurement,LCMeasurement,CCMeasurement,RCMeasurement,REMeasurement
VG1,17/10/2026 19:46:33,1981,29,-0.4,17.02,17.31,17.3,17.25,16.54
```
RunMinute ghi dạng số (không có `="..."`).

**File đang mở trong Excel:** Windows khóa file nên không ghi được. Chương trình báo, giữ dữ liệu lại (lưu cả xuống `fpp_pending.json` để không mất khi tắt máy) và tự ghi bù ngay khi file được đóng. Các file khác vẫn được ghi bình thường.

## Các file cạnh exe

| File | Nội dung |
|---|---|
| `fpp_config.json` | Thư mục lưu, Line chọn lần cuối, máy dùng lần cuối, **Offset Nagy**, định dạng CSV. Tự tạo. |
| `fpp_pending.json` | Dữ liệu đang chờ ghi bù (chỉ có khi có file bị khóa). |
| `fpp_catalog.json` | Không bắt buộc. Nếu có, dùng thay danh mục Line / Product / Test nhúng sẵn (xem `src/catalog.json`), để thêm sản phẩm không cần build lại. |

`calibration.txt` (hệ số đa thức 4PP `A B C Low High`) vẫn nằm trong thư mục lưu như bản gốc, nên dùng lại được file cũ.

**Lần đầu chạy trên máy đang dùng bản cũ:** bản gốc lưu cài đặt trong hồ sơ Windows của người dùng nên không chuyển sang được. Cần làm lại:
1. **Save Folder → Choose Save Folder...** (mặc định `C:\Data\`).
2. Với Nagy: menu **Offset Calibration**, nhập offset đang dùng (ví dụ `-0.4`) và mật khẩu.
3. **CSV Format**: chọn Default / ERS / Sequoia.

## Giữ nguyên như bản gốc

- Danh mục 8 line, 30 sản phẩm (4PP) và Nagy có thêm sản phẩm **TZ18** (test 5 Points / 3 Points); còn lại, 14 test, 220 điểm đo (4PP) và 4 line, 10 sản phẩm, 3 test (Nagy), lấy nguyên từ code gốc.
- 4PP: chờ nhấc / đặt đầu đo (2 lần đọc lệch < 1%, test Calibration < 10%), quét 4 mức dòng × 3 lần đọc điện áp, hồi quy, loại khi r < 0.99, hệ số Sample Size (Float Ribbon 1.0 / 4x4 0.9981 / 12x12 0.9997 / 3x3 0.9966), đa thức hiệu chỉnh trong khoảng Low–High. Test "2 Inch / 4 Inch Ohms" dùng giá trị điện trở trung bình của Keithley.
- Kết quả 4PP có điểm "Not Stored" (Continuous, Gage R and R, Calibration) ghi vào `C:\Records\` như cũ (đổi được bằng `records_path` trong `fpp_config.json`).
- Line OT1: **Get Host Data** đọc `http://ot01_h1/reports/samples/sample.flaws.00`.
- Bấm vào một kết quả để đo lại điểm đó.

## Khác với bản gốc

- Giao diện theo bản cũ: Setup Screen nhỏ gọn, MeasureForm với kết quả bên trái, số đo và biểu đồ bên phải.
- Đổi Offset Nagy phải nhập mật khẩu (`ZZCARROT`, khai báo ở `OFFSET_PASSWORD` trong `src/main.rs`).
- Sample Size chỉ còn một mục **10cm x 10cm** (hệ số 0.9981, bằng mẫu 4x4 inch của bản gốc). Không ảnh hưởng Nagy; với máy 4 điểm, các test trước đây dùng "Float Ribbon" (hệ số 1.0) nay nhân 0.9981.
- `calibration.txt` do bản cũ ghi (có BOM UTF-8) đọc được bình thường; cảnh báo về file này chỉ hiện ở màn hình 4-Point Probe.

- Thêm chọn định dạng CSV Default / ERS / Sequoia.
- Dòng CSV không còn dấu phẩy thừa ở cuối, RunMinute ghi dạng số, khớp với file mẫu.
- Sửa lỗi: đo lại một điểm **giữa chừng** làm bản gốc treo (4PP) hoặc ghi tên điểm thành "Continue" (Nagy). Bản mới đo lại xong thì tiếp tục đúng điểm đang đo.
- Không có cổng COM hoặc không tìm thấy máy: hiện thông báo và nút **Retry connection** thay vì tự thoát.
- Hỏi xác nhận khi bấm Exit Measurement mà chưa Submit.

## Kiểm thử

```
cargo test
```
26 bài kiểm thử: đọc số kiểu VB, định dạng CSV và tên file 3 định dạng, ghi bù khi file bị khóa, giao thức Keithley / Nagy, và toàn bộ vòng đo với máy giả lập (đo lại, offset, Continuous, lỗi hồi quy).
