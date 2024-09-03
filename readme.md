# vitepress-pdf-export
This program does one thing well, export a VitPress site as nice PDF with correct links and page numbers. Additionaly we expect `vitepress-pdf-export` to be run as part of a CI actions so all options are handled by a toml configuration file.

## Status
* [x] Fetches the latest Chrome Build
* [X] Enumerate URLS from Single URL
* [X] Use Cached Chrome Build
* [X] Renders each URL into a PDF file
  * [X] TempDir for PDF
* [X] Merges the PDFs into a Single PDF
  * [X] with Outline
* [X] Rewrites links between the URLS into PDF
* [X] Add Merge Tests
* [ ] Add Page Numbers

## Supported Platforms
Currently only `MacOS`, `MacOS Arm`, and `Linux` are supported.

## Config
Key               | Description                                                                                     | Default | Type
------------------|-------------------------------------------------------------------------------------------------|---------|-----------------
`chrome_cache`    | Directory used to download and cache chrome builds                                              | "/tmp"  | `PathBuf`
`chrome_version`  | Pin Chrome to a specfic revision, e.g., `1336641`. If unset we use that latest known good build | `None`  | `Option<String>`
`output_pdf`      | The merged PDF file                                                                             |         | `PathBuf`
`url`             | VitePress URl.  e.g., `http://localhost:5173`                                                   |         | `String`
`vitepress_links` | Paths to json file defining the url layout of the VitePress site                                |         | `Vec<PathBuf>`

### page_number
Key     | Description                                               | Type
--------|-----------------------------------------------------------|--------------------------------------------------------
`color` | RGB values between 0 and 1.0                              | TOML Table with keys `r`, `g`, `b` and values are `f64`
`font`  | PDF Type 1 - see table below for options                  | `String`
`size`  | Font size                                                 | `i16`
`x`     | Page Number X offset (in inches) from the top left corner | `f64`
`y`     | Page Number Y offset (in inches) from the top left corner | `f64`

#### PDF Type 1 Fonts
* `Times−Roman`, `Times−Bold`, `Times−Italic`, `Times−BoldItalic`,
* `Helvetica`, `Helvetica−Bold`, `Helvetica−Oblique`, `Helvetica−BoldOblique`,
* `Courier`, `Courier−Bold`, `Courier−Oblique`, `Courier−BoldOblique`

### print_to_pdf
Yes underscore is the default case for TOML but these options come from Chrome DevTool Protocol which uses camel case. See [Chrome DevTool Protocol](https://chromedevtools.github.io/devtools-protocol/tot/Page/#method-printToPDF) for additional options.
Key                       | Description                                               | Type           | Chrome DevTool Protocol Default
--------------------------|-----------------------------------------------------------|----------------|--------------------------------
`generateDocumentOutline` | Whether or not to embed the document outline into the PDF | `Option<bool>` | False
`marginBottom`            | Bottom margin in inches                                   | `Option<f64>`  | Defaults to 1cm (~0.4 inches)
`marginLeft`              | Left margin in inches                                     | `Option<f64>`  | Defaults to 1cm (~0.4 inches)
`marginRight`             | Right margin in inches                                    | `Option<f64>`  | Defaults to 1cm (~0.4 inches)
`marginTop`               | Top margin in inches                                      | `Option<f64>`  | Defaults to 1cm (~0.4 inches)
`paperHeight`             | Paper height in inches                                    | `Option<f64>`  | Defaults to 8.5 inches
`paperWidth`              | Paper width in inches                                     | `Option<f64>`  | Defaults to 11.0 inches
`printBackground`         | Print background graphics                                 | `Option<bool>` | False

## Useful Dev Resources
* [PDF 1.7 Spec](https://opensource.adobe.com/dc-acrobat-sdk-docs/pdfstandards/PDF32000_2008.pdf)
* [PDF Reference](https://opensource.adobe.com/dc-acrobat-sdk-docs/pdfstandards/pdfreference1.4.pdf)
* *PDF Explained: The ISO Standard for Document Exchange* by John Whitington
* [PDF Debugger](https://pdf.hyzyla.dev/): Web based, easier to navigate but doesn't have support for Page Contents.
* [Interactive PDF Analysis](https://github.com/seekbytes/IPA): Native app that you have to compile, navigation is a little harder but it has support for Page Contents.