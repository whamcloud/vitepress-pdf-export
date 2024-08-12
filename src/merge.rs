use crate::Config;
use anyhow::{anyhow, Result};
use indexmap::IndexMap;
use lopdf::{Dictionary, Document, Object, ObjectId};
use std::collections::BTreeMap;
use std::path::PathBuf;

pub fn rewrite_links(doc: &mut Document, url_to_page_num: IndexMap<String, usize>) -> Result<()> {
    // Build a maping from URL to Page ID
    let page_num_to_id = doc.get_pages();
    let mut url_to_page_id = IndexMap::new();
    for (url, page_num) in url_to_page_num {
        let page_num: u32 = page_num as u32 + 1; // Get Pages starts indexing at 1
        url_to_page_id.insert(url, page_num_to_id.get(&page_num).unwrap());
    }

    let mut to_rewrite: Vec<(ObjectId, ObjectId)> = vec![];

    let mut named_destinations = IndexMap::new();
    let catalog: &Dictionary = doc.catalog()?;
    let mut tree = doc.get_dict_in_dict(catalog, b"Dests");
    if tree.is_err() {
        let names = doc.get_dict_in_dict(catalog, b"Names");
        if let Ok(names) = names {
            let dests = doc.get_dict_in_dict(names, b"Dests");
            if dests.is_ok() {
                tree = dests;
            }
        }
    }

    if let Ok(tree) = tree {
        // FIXME tree looks fine but named_destinations is empty
        doc.get_named_destinations(tree, &mut named_destinations)?;
    }

    for page_id in doc.page_iter() {
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

        for (annotation_id, annotation) in annotations {
            let subtype = annotation
                .get_deref(b"Subtype", doc)
                .and_then(Object::as_name_str)
                .unwrap_or("");

            if subtype.eq("Link") {
                if let Ok(ahref) = annotation.get_deref(b"A", doc).and_then(Object::as_dict) {
                    let mut url = ahref.get(b"URI")?.as_string()?.to_string();
                    let parts: Vec<&str> = url.split('/').collect();
                    let page = parts.last().unwrap();

                    // HTML anchor are complied to PDF named destinations
                    if page.contains('#') {
                        let _page = page.split('#').last().unwrap();
                        // FIXME - Named Destinations is currently empty
                    } else {
                        // For URLS that end in "/", a.k.a without a page, we set the page to index.html
                        if page.is_empty() {
                            url.push_str("index.html")
                        }
                        if let Some(page_id) = url_to_page_id.get(&url) {
                            to_rewrite.push((annotation_id, **page_id));
                        }
                    }
                }
            }
        }
    }

    for (annotation_id, page_id) in to_rewrite {
        let annot = doc.get_dictionary_mut(annotation_id)?;
        annot.remove(b"A");
        annot.set("Dest", Object::from(vec![page_id.into(), "Fit".into()]));
    }
    Ok(())
}

pub struct PdfParts {
    objects: BTreeMap<ObjectId, Object>,
    pages: BTreeMap<ObjectId, Object>,
}

// Loads pdf into memory and merges the PDF objects
fn load_pdfs_into_parts(
    url_to_pdf_path: IndexMap<String, PathBuf>,
) -> Result<(PdfParts, IndexMap<String, usize>)> {
    // Used remap links internal to the VitePress site to internal PDF links
    let mut url_to_page_num = IndexMap::new();

    // Go through all PDFs and collect all pages and objects which we use to generate a merged PDF
    let mut objects = BTreeMap::new();
    let mut pages = BTreeMap::new();
    let mut starting_id = 1;

    for (url, path) in url_to_pdf_path {
        let mut doc = Document::load(path)?;

        // Record the page where a PDF generate from `url` are inserted into the merged PDF
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

pub fn merge_pdfs_objects(parts: &PdfParts) -> Result<Document> {
    // Catalog and Pages are mandatory
    let mut catalog_object: Option<(ObjectId, Object)> = None;
    let mut outlines = vec![];
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
            "Outline" => {} // Ignored, not supported yet
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

    let mut state: Option<(ObjectId, Dictionary, ObjectId)> = None;
    if !outlines.is_empty() {
        let mut count = 0;

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
    }

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
        match state {
            None => (),
            Some((parent_id, _, _)) => {
                dictionary.set(b"Outlines", parent_id);
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

pub fn merge_pdfs(conf: &Config, url_to_pdf_path: IndexMap<String, PathBuf>) -> Result<()> {
    let (parts, url_to_page_num) = load_pdfs_into_parts(url_to_pdf_path)?;
    let mut pdf = merge_pdfs_objects(&parts)?;
    rewrite_links(&mut pdf, url_to_page_num)?;
    pdf.save(&conf.output_pdf)?;
    println!("Merged PDF is avalible here {}", conf.output_pdf.display());
    Ok(())
}

// This function is useful for writing code / debuging
#[allow(dead_code)]
fn print_outline(
    doc: &lopdf::Document,
    outline_obj: &lopdf::Dictionary,
    level: usize,
) -> Result<()> {
    let title = match outline_obj.get(b"Title") {
        Ok(t) => t.as_string()?.to_string(),
        Err(_) => "Outline Dictionary".to_string(),
    };

    println!("{}{title}", "  ".repeat(level));
    if let Ok(first) = outline_obj.get(b"First") {
        let first = doc.get_object(first.as_reference()?)?.as_dict()?;
        print_outline(doc, first, level + 1)?;
    }

    if let Ok(next) = outline_obj.get(b"Next") {
        let next = doc.get_object(next.as_reference()?)?.as_dict()?;
        print_outline(doc, next, level)?;
    }

    Ok(())
}
