// Copyright (c) 2024 DDN. All rights reserved.
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file.

use crate::Config;
use anyhow::{anyhow, Result};
use indexmap::IndexMap;
use lopdf::{
    content::{Content, Operation},
    dictionary, Dictionary, Document, Object, ObjectId,
};
use std::{collections::BTreeMap, path::PathBuf, process::ExitCode};

struct PdfParts {
    objects: BTreeMap<ObjectId, Object>,
    pages: BTreeMap<ObjectId, Object>,
}

/// Loads PDFs into memory as PDF Objects and merges the PDF Objects
fn merge_pdf_objects(
    url_to_pdf_doc: IndexMap<String, Document>,
) -> Result<(PdfParts, IndexMap<String, usize>)> {
    // Used remap links internal to the VitePress site to internal PDF links
    let mut url_to_page_num = IndexMap::new();

    // Go through all PDFs and collect all pages and objects which we use to generate a merged PDF
    let mut objects = BTreeMap::new();
    let mut pages = BTreeMap::new();
    let mut starting_id = 1;

    for (url, mut doc) in url_to_pdf_doc {
        // Record the page where a PDF generate from `url` are inserted into the merged PDF.
        // Used by `rewrite_vitepress_links`.
        url_to_page_num.insert(url.clone(), pages.len());

        // Object IDs are indexes not UUIDs so we need to renumber them
        // before inserting them into a unified collection.
        doc.renumber_objects_with(starting_id);
        starting_id = doc.max_id + 1;

        pages.extend(
            doc.get_pages()
                .into_values()
                .map(|object_id| (object_id, doc.get_object(object_id).unwrap().to_owned()))
                .collect::<BTreeMap<ObjectId, Object>>(),
        );
        objects.extend(doc.objects);
    }

    Ok((PdfParts { objects, pages }, url_to_page_num))
}

fn build_pdf_from_objects(parts: &PdfParts) -> Result<Document> {
    // Catalog and Pages are mandatory
    let mut catalog_object: Option<(ObjectId, Object)> = None;
    let mut outlines: Vec<((u32, u16), Dictionary)> = vec![];
    let mut pages_object: Option<(ObjectId, Object)> = None;

    // Process all objects except "Page" type
    let mut document = Document::with_version("1.5");
    for (object_id, object) in parts.objects.iter() {
        match object.type_name().unwrap_or("") {
            "Catalog" => {
                // Collect a first "Catalog" object and use it for the future "Pages"
                catalog_object = Some((
                    if let Some((id, _)) = catalog_object {
                        id
                    } else {
                        *object_id
                    },
                    object.clone(),
                ));
            }
            "Pages" => {
                // Collect and update a first "Pages" object and use it for the future "Catalog"
                // We have also to merge all dictionaries of the old and the new "Pages" object
                if let Ok(dictionary) = object.as_dict() {
                    let mut dictionary = dictionary.clone();
                    if let Some((_, ref object)) = pages_object {
                        if let Ok(old_dictionary) = object.as_dict() {
                            dictionary.extend(old_dictionary);
                        }
                    }

                    pages_object = Some((
                        if let Some((id, _)) = pages_object {
                            id
                        } else {
                            *object_id
                        },
                        Object::Dictionary(dictionary),
                    ));
                }
            }
            "Page" => {} // Ignored, processed later and separately
            "Outlines" => {
                // Saved seperately and processed later
                outlines.push((*object_id, object.as_dict()?.clone()));
            }
            _ => {
                document.objects.insert(*object_id, object.clone());
            }
        }
    }

    // If no "Pages" found abort
    if pages_object.is_none() {
        return Err(anyhow!("Pages root not found."));
    }

    // Iter over all "Page" and collect with the parent "Pages" created before
    for (object_id, object) in parts.pages.iter() {
        if let Ok(dictionary) = object.as_dict() {
            let mut dictionary = dictionary.clone();
            dictionary.set("Parent", pages_object.as_ref().unwrap().0);

            document
                .objects
                .insert(*object_id, Object::Dictionary(dictionary));
        }
    }

    // Merge any "Outlines" into a single "Outlines"
    let outlines_id = merge_outlines(&mut document, outlines)?;

    // If no "Catalog" found abort
    if catalog_object.is_none() {
        return Err(anyhow!("Catalog root not found."));
    }

    let catalog_object = catalog_object.unwrap();
    let pages_object = pages_object.unwrap();

    // Build a new "Pages" with updated fields
    if let Ok(dictionary) = pages_object.1.as_dict() {
        let mut dictionary = dictionary.clone();

        // Set new pages count
        dictionary.set("Count", parts.pages.len() as u32);

        // Set new "Kids" list (collected from documents pages) for "Pages"
        dictionary.set(
            "Kids",
            parts
                .pages
                .clone()
                .into_keys()
                .map(Object::Reference)
                .collect::<Vec<_>>(),
        );

        document
            .objects
            .insert(pages_object.0, Object::Dictionary(dictionary));
    }

    // Build a new "Catalog" with updated fields
    if let Ok(dictionary) = catalog_object.1.as_dict() {
        let mut dictionary = dictionary.clone();
        dictionary.set("Pages", pages_object.0);
        match outlines_id {
            None => (),
            Some(id) => {
                dictionary.set(b"Outlines", id);
            }
        }
        document
            .objects
            .insert(catalog_object.0, Object::Dictionary(dictionary));
    }

    document.trailer.set("Root", catalog_object.0);

    // Update the max internal ID as wasn't updated before due to direct objects insertion
    document.max_id = document.objects.len() as u32;

    // Reorder all new Document objects
    document.renumber_objects();

    //Set any Bookmarks to the First child if they are not set to a page
    document.adjust_zero_pages();

    document.compress();

    Ok(document)
}

fn merge_outlines(
    document: &mut Document,
    outlines: Vec<((u32, u16), Dictionary)>,
) -> Result<Option<ObjectId>> {
    if !outlines.is_empty() {
        let mut count = 0;
        let mut state: Option<(ObjectId, Dictionary, ObjectId)> = None;
        for (outline_id, outline_obj) in outlines {
            count += outline_obj.get(b"Count")?.as_i64()?;

            state = match state {
                None => {
                    let last_item = outline_obj.get(b"Last")?.as_reference()?;
                    Some((outline_id, outline_obj, last_item))
                }
                Some((parent_id, parent_obj, last_item_id)) => {
                    let last_item = document.get_object_mut(last_item_id)?.as_dict_mut()?;
                    last_item.set(b"Next", outline_obj.get(b"First")?.as_reference()?);

                    let mut first_item = document
                        .get_object_mut(outline_obj.get(b"First")?.as_reference()?)?
                        .as_dict_mut()?;

                    first_item.set(b"Prev", last_item_id);

                    while first_item.has(b"Next") {
                        first_item.set(b"Parent", parent_id);
                        first_item = first_item.get_mut(b"Next")?.as_dict_mut()?;
                    }
                    first_item.set(b"Parent", parent_id);

                    Some((
                        parent_id,
                        parent_obj,
                        outline_obj.get(b"Last")?.as_reference()?,
                    ))
                }
            };
        }
        let (parent_id, mut parent_obj, last_item_id) = state.clone().unwrap();
        parent_obj.set("Count", Object::from(count));
        parent_obj.set("Last", last_item_id);
        document
            .objects
            .insert(parent_id, lopdf::Object::Dictionary(parent_obj));
        return Ok(Some(parent_id));
    }
    Ok(None)
}

fn rewrite_vitepress_links(
    conf: &Config,
    doc: &mut Document,
    url_to_page_num: IndexMap<String, usize>,
) -> Result<Vec<String>> {
    // Build a maping from URL to Page ID
    let page_num_to_id = doc.get_pages();
    let mut url_to_page_id = IndexMap::new();
    for (url, page_num) in url_to_page_num {
        let page_num: u32 = page_num as u32 + 1; // Get Pages starts indexing at 1
        url_to_page_id.insert(url, page_num_to_id.get(&page_num).unwrap());
    }

    let mut problem_urls = vec![];
    let mut to_rewrite: Vec<(ObjectId, ObjectId)> = vec![];

    // Go through the pages
    for page_id in doc.page_iter() {
        // Get the Annoation ID and Object
        let mut annotations: Vec<(ObjectId, &Dictionary)> = vec![];
        if let Ok(page) = doc.get_dictionary(page_id) {
            match page.get(b"Annots") {
                Ok(Object::Reference(ref id)) => doc
                    .get_object(*id)
                    .and_then(Object::as_array)
                    .unwrap()
                    .iter()
                    .flat_map(Object::as_reference)
                    .flat_map(|id| doc.get_dictionary(id))
                    .for_each(|a| annotations.push((*id, a))),

                Ok(Object::Array(ref array)) => {
                    for object in array {
                        let id = object.as_reference()?;
                        annotations.push((id, doc.get_dictionary(id)?));
                    }
                }
                _ => {}
            }
        }

        // We go through found annotations
        for (annotation_id, annotation) in annotations {
            let subtype = annotation
                .get_deref(b"Subtype", doc)
                .and_then(Object::as_name_str)
                .unwrap_or("");

            if subtype.eq("Link") {
                // We've found a Annotation Link with an URL
                if let Ok(ahref) = annotation.get_deref(b"A", doc).and_then(Object::as_dict) {
                    let mut url = ahref.get(b"URI")?.as_string()?.to_string();
                    let parts: Vec<&str> = url.split('/').collect();
                    let page = parts.last().unwrap();

                    // For URLS that end in "/", a.k.a without a page, we set the page to index.html
                    if page.is_empty() {
                        url.push_str("index.html")
                    }

                    match url_to_page_id.get(&url) {
                        Some(page_id) => to_rewrite.push((annotation_id, **page_id)),
                        None => {
                            // We only care about
                            if url.starts_with(&conf.url) {
                                problem_urls.push(url);
                            }
                        }
                    }
                }
            }
        }
    }

    for (annotation_id, page_id) in to_rewrite {
        let annot = doc.get_dictionary_mut(annotation_id)?;
        // Delete the external Link
        annot.remove(b"A");
        // Insert the internal Page Destination
        annot.set("Dest", Object::from(vec![page_id.into(), "Fit".into()]));
    }

    Ok(problem_urls)
}

fn add_page_numbers(doc: &mut Document, conf: &Config) -> Result<()> {
    if let Some(style) = &conf.page_number {
        // Add the font for each page to reference
        let font_id = doc.add_object(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => style.font.to_string(),
        });

        // Go through each page
        let pages = doc.get_pages();
        for (page_num, page_id) in pages {
            let mut font_num = 1;
            // Get pages Resouces
            if let Ok(page) = doc.get_dictionary_mut(page_id) {
                if let Ok(resource_dict) =
                    page.get_mut(b"Resources").map(|o| o.as_dict_mut().unwrap())
                {
                    // Get the pages fonts
                    if let Ok(fonts) = resource_dict
                        .get_mut(b"Font")
                        .map(|o| o.as_dict_mut().unwrap())
                    {
                        // Find the first unused font index - this is normally F1
                        while fonts.has(format!("F{font_num}").as_bytes()) {
                            font_num += 1;
                        }
                        fonts.set(format!("F{font_num}").as_bytes(), font_id);
                    }
                }
            }

            let content: Content = Content {
                operations: vec![
                    // Begin Text Element
                    Operation::new("BT", vec![]),
                    // Font Color
                    Operation::new(
                        "rg",
                        vec![
                            style.color.r.into(),
                            style.color.g.into(),
                            style.color.b.into(),
                        ],
                    ),
                    // Font and Size
                    Operation::new("Tf", vec![format!("F{font_num}").into(), style.size.into()]),
                    // Set the text matrix, this is an affine transformation matrix which is used to veritically filp the text
                    // and position it at the bottom of the page. The Vertical filp is required by due to how chrome renders the PDFs.
                    // See section 4.2.2 in PDF Reference for more details.
                    Operation::new(
                        "Tm",
                        vec![
                            (1).into(),
                            0.into(),
                            0.into(),
                            (-1).into(),
                            (style.x * 300.0).into(), // Convert x from inches into dots by multplying by the standard 300 DPI
                            (style.y * 300.0).into(),
                        ],
                    ),
                    // Set the page number text
                    Operation::new(
                        "Tj",
                        vec![Object::string_literal(format!("Page {}", page_num))],
                    ),
                    // End Text
                    Operation::new("ET", vec![]),
                ],
            };
            doc.add_to_page_content(page_id, content)?;
        }
    }

    Ok(())
}

pub fn merge_pdfs(conf: &Config, url_to_pdf_path: IndexMap<String, PathBuf>) -> Result<ExitCode> {
    let mut url_to_pdf_doc = IndexMap::new();
    for (url, path) in url_to_pdf_path {
        url_to_pdf_doc.insert(url.clone(), Document::load(path)?);
    }

    let (parts, url_to_page_num) = merge_pdf_objects(url_to_pdf_doc)?;

    let mut pdf = build_pdf_from_objects(&parts)?;

    let problem_urls = rewrite_vitepress_links(conf, &mut pdf, url_to_page_num)?;

    add_page_numbers(&mut pdf, conf)?;

    pdf.save(&conf.output_pdf)?;

    println!("Merged PDF is avalible here {}", conf.output_pdf.display());
    if !problem_urls.is_empty() {
        println!(
            "Unable to remap these VitePress links.\n{}",
            problem_urls.join("\n")
        );
        return Ok(ExitCode::FAILURE);
    }

    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use headless_chrome::types::PrintToPdfOptions;
    use indexmap::IndexSet;
    use lopdf::{
        content::{Content, Operation},
        dictionary, Stream,
    };

    pub fn generate_pdf_with_link(url: String) -> Document {
        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();
        let font_id = doc.add_object(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => "Courier",
        });
        let resources_id = doc.add_object(dictionary! {
            "Font" => dictionary! {
                "F1" => font_id,
            },
        });
        let content: Content = Content {
            operations: vec![
                Operation::new("BT", vec![]),
                Operation::new("Tf", vec!["F1".into(), 48.into()]),
                Operation::new("Td", vec![100.into(), 600.into()]),
                Operation::new("Tj", vec![Object::string_literal("Hello World!")]),
                Operation::new("ET", vec![]),
            ],
        };
        let content_id = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
        let a_id = doc.add_object(dictionary! {
            "Type" => "Action",
            "S" => "URI",
            "URI" => Object::string_literal(url),
        });
        let annot_id = doc.add_object(dictionary! {
            "Type" => "Annot",
            "Subtype" => "Link",
            "Rect" => vec![0.into(), 0.into(), 595.into(), 842.into()],
            "F" => 4,
            "Border" => vec![1.into(), 1.into(), 1.into()],
            "A" => a_id,
        });

        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Contents" => content_id,
            "Resources" => resources_id,
            "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
            "Annots" => vec![annot_id.into()],
        });
        let pages = dictionary! {
            "Type" => "Pages",
            "Kids" => vec![page_id.into()],
            "Count" => 1,
        };
        doc.objects.insert(pages_id, Object::Dictionary(pages));
        let catalog_id = doc.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });
        doc.trailer.set("Root", catalog_id);

        doc
    }

    pub fn generate_pdf_with_outline() -> Document {
        let mut doc = Document::with_version("1.7");
        let pages_id = doc.new_object_id();
        let font_id = doc.add_object(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => "Courier",
        });
        let resources_id = doc.add_object(dictionary! {
            "Font" => dictionary! {
                "F1" => font_id,
            },
        });
        let content: Content = Content {
            operations: vec![
                Operation::new("BT", vec![]),
                Operation::new("Tf", vec!["F1".into(), 48.into()]),
                Operation::new("Td", vec![100.into(), 600.into()]),
                Operation::new("Tj", vec![Object::string_literal("Hello World!")]),
                Operation::new("ET", vec![]),
            ],
        };
        let content_id = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));

        let outline_dict = doc.new_object_id();
        let node_1 = doc.new_object_id();
        let node_2 = doc.new_object_id();
        let node_3 = doc.new_object_id();

        // outline_dictionary
        //  └► node 1
        //    └► node 2
        //    └► node 3

        doc.objects.insert(
            outline_dict,
            Object::Dictionary(dictionary! {
                "Type" => "Outlines",
                "First" => node_1,
                "Last" => node_1,
                "Count" => 3,
            }),
        );

        doc.objects.insert(
            node_1,
            Object::Dictionary(dictionary! {
                "Title" => Object::string_literal("Node 1"),
                "Parent" => outline_dict,
                "First" => node_2,
                "Last" => node_3,
                "Count" => 2
            }),
        );

        doc.objects.insert(
            node_2,
            Object::Dictionary(dictionary! {
                "Title" => Object::string_literal("Node 2"),
                "Parent" => node_1,
                "Next" => node_3,
                "Count" => 1,
            }),
        );

        doc.objects.insert(
            node_3,
            Object::Dictionary(dictionary! {
                "Title" => Object::string_literal("Node 3"),
                "Parent" => node_1,
                "Prev" => node_2,
                "Count" => 1,
            }),
        );

        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Contents" => content_id,
            "Resources" => resources_id,
            "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
        });

        let pages = dictionary! {
            "Type" => "Pages",
            "Kids" => vec![page_id.into()],
            "Count" => 1,
        };
        doc.objects.insert(pages_id, Object::Dictionary(pages));
        let catalog_id = doc.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
            "Outlines" => outline_dict,
        });
        doc.trailer.set("Root", catalog_id);

        doc
    }

    #[test]
    fn test_merge_toc() {
        let mut map = IndexMap::new();
        map.insert(
            "http://example.com/1.html".to_string(),
            generate_pdf_with_outline(),
        );
        map.insert(
            "http://example.com/2.html".to_string(),
            generate_pdf_with_outline(),
        );
        map.insert(
            "http://example.com/3.html".to_string(),
            generate_pdf_with_outline(),
        );

        let (parts, _) = merge_pdf_objects(map).unwrap();

        let pdf = build_pdf_from_objects(&parts).unwrap();

        //let mut  pdf = generate_pdf_with_outline();
        let cat = pdf.catalog().unwrap();
        insta::assert_debug_snapshot!(outline(
            &pdf,
            pdf.get_object(cat.get(b"Outlines").unwrap().as_reference().unwrap())
                .unwrap()
                .as_dict()
                .unwrap()
        ));
    }

    // This tests re-writing URLs to PDF Destinations
    // 1. We generate 3 pdfs each of which have a link to the next
    // 2. We merged the pdfs together.
    // 3. We assert that the 3rd pdf's was not remapped.
    // 4. We go through the pages and ensure the Annotation points to the next page
    #[test]
    fn test_rewrite_urls() {
        let conf = Config {
            chrome_cache: PathBuf::new(),
            chrome_version: None,
            output_pdf: PathBuf::new(),
            url: "http://example.com".to_string(),
            urls: IndexSet::new(),
            vitepress_links: Vec::new(),
            page_number: None,
            print_to_pdf: PrintToPdfOptions::default(),
        };
        let mut map = IndexMap::new();
        map.insert(
            "http://example.com/1.html".to_string(),
            generate_pdf_with_link("http://example.com/2.html".to_string()),
        );
        map.insert(
            "http://example.com/2.html".to_string(),
            generate_pdf_with_link("http://example.com/3.html".to_string()),
        );
        map.insert(
            "http://example.com/3.html".to_string(),
            generate_pdf_with_link("http://example.com/4.html".to_string()),
        );

        let (parts, url_to_page_num) = merge_pdf_objects(map).unwrap();

        let mut pdf = build_pdf_from_objects(&parts).unwrap();

        let problem_urls = rewrite_vitepress_links(&conf, &mut pdf, url_to_page_num).unwrap();

        assert_eq!(problem_urls, vec!["http://example.com/4.html".to_string()]);

        let page_num_to_id = pdf.get_pages();
        for (page_num, page_id) in pdf.page_iter().enumerate() {
            // Only the first two pages are remapped
            if page_num == page_num_to_id.len() - 1 {
                break;
            }

            let annotations = pdf.get_page_annotations(page_id).unwrap();

            // Assert that there's no stray annotations
            assert_eq!(annotations.len(), 1);

            let annotation = annotations.first().unwrap();

            let dest = annotation.get_deref(b"Dest", &pdf).unwrap();

            let next_page_id = page_num_to_id.get(&(page_num as u32 + 2)).unwrap(); // +2 because page_num is 0 index and get_pages is 1 indexed.

            assert_eq!(
                *next_page_id,
                dest.as_array()
                    .unwrap()
                    .first()
                    .unwrap()
                    .as_reference()
                    .unwrap()
            );
        }
    }

    #[derive(Eq, Debug, Hash, PartialEq)]
    struct Node {
        title: String,
        prev: Option<String>,
        next: Option<String>,
        parent: Option<String>,
        childern: Vec<tests::Node>,
    }

    fn outline(doc: &lopdf::Document, outline_obj: &lopdf::Dictionary) -> Result<Node> {
        let title = match outline_obj.get(b"Title") {
            Ok(t) => t.as_string()?.to_string(),
            Err(_) => "Outline Dictionary".to_string(),
        };
        let prev: Option<String> = match outline_obj.get(b"Prev") {
            Ok(t) => Some(
                doc.get_dictionary(t.as_reference()?)?
                    .get(b"Title")?
                    .as_string()?
                    .to_string(),
            ),
            Err(_) => None,
        };

        let next: Option<String> = match outline_obj.get(b"Next") {
            Ok(t) => Some(
                doc.get_dictionary(t.as_reference()?)?
                    .get(b"Title")?
                    .as_string()?
                    .to_string(),
            ),
            Err(_) => None,
        };

        let parent: Option<String> = match outline_obj.get(b"Parent") {
            Ok(t) => {
                let parent = doc.get_dictionary(t.as_reference()?)?;
                Some(match parent.get(b"Title") {
                    Ok(t) => t.as_string()?.to_string(),
                    Err(_) => "Outline Dictionary".to_string(),
                })
            }
            Err(_) => None,
        };

        let mut childern = vec![];
        if let Ok(child) = outline_obj.get(b"First") {
            let mut child = doc.get_object(child.as_reference()?)?.as_dict()?;

            childern.push(outline(doc, child)?);

            while child.has(b"Next") {
                let child_id = child.get(b"Next")?.as_reference()?;

                child = doc.get_object(child_id)?.as_dict()?;

                childern.push(outline(doc, child)?);
            }
        }

        Ok(Node {
            title,
            prev,
            next,
            parent,
            childern,
        })
    }
}
