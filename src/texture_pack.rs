use std::collections::HashMap;
use std::os::raw::c_void;

use image::{DynamicImage, GenericImageView};

use crate::block_texture_faces::BlockFaces;
use crate::chunk::BlockID;
use crate::constants::{BLOCK_TEXTURE_SIZE, TEXTURE_ATLAS_SIZE};
use crate::types::UVCoords;

pub fn generate_texture_atlas() -> (u32, HashMap<BlockID, BlockFaces<UVCoords>>) {
    let face_images = create_face_images_map();
    let atlas = create_texture_atlas(TEXTURE_ATLAS_SIZE as i32);
    let face_uvs = create_face_uvs_map(atlas, face_images);
    (atlas, face_uvs)
}

fn create_face_images_map() -> HashMap<BlockID, BlockFaces<&'static str>> {
    let mut face_images: HashMap<BlockID, BlockFaces<&str>> = HashMap::new();
    face_images.insert(BlockID::Dirt, BlockFaces::All("blocks/dirt.png"));
    face_images.insert(BlockID::GrassBlock, BlockFaces::Sides {
        sides: "blocks/grass_block_side.png",
        top: "blocks/grass_block_top.png",
        bottom: "blocks/dirt.png",
    });
    face_images.insert(BlockID::Cobblestone, BlockFaces::All("blocks/cobblestone.png"));
    face_images.insert(BlockID::Obsidian, BlockFaces::All("blocks/obsidian.png"));
    face_images.insert(BlockID::OakLog, BlockFaces::Sides {
        sides: "blocks/oak_log.png",
        top: "blocks/oak_log_top.png",
        bottom: "blocks/oak_log_top.png",
    });
    face_images.insert(BlockID::OakLeaves, BlockFaces::All("blocks/oak_leaves_mod.png"));
    face_images.insert(BlockID::Urss, BlockFaces::All("blocks/urss.png"));
    face_images.insert(BlockID::Hitler, BlockFaces::All("blocks/hitler.png"));
    face_images.insert(BlockID::Debug, BlockFaces::All("blocks/debug.png"));
    face_images.insert(BlockID::Debug2, BlockFaces::All("blocks/debug2.png"));
    face_images
}

/// Create and allocate storage for a texture of size (atlas_size * atlas_size)
fn create_texture_atlas(atlas_size: i32) -> u32 {
    let mut atlas: u32 = 0;
    gl_call!(gl::CreateTextures(gl::TEXTURE_2D, 1, &mut atlas));
    gl_call!(gl::TextureParameteri(atlas, gl::TEXTURE_MIN_FILTER, gl::NEAREST_MIPMAP_NEAREST as i32));
    gl_call!(gl::TextureParameteri(atlas, gl::TEXTURE_MAG_FILTER, gl::NEAREST as i32));
    gl_call!(gl::TextureStorage2D(atlas, 1, gl::RGBA8, atlas_size, atlas_size));
    atlas
}

fn create_face_uvs_map(atlas: u32, face_images: HashMap<BlockID, BlockFaces<&str>>) -> HashMap<BlockID, BlockFaces<UVCoords>> {
    // Load all the images and fill the UV map for all the blocks
    // TODO don't load the same texture multiple times if reused for another block

    let mut x = 0;
    let mut y = 0;

    // Puts an image into the texture atlas at (x, y)
    let mut put_image_into_atlas = |image: &DynamicImage| {
        let (dest_x, dest_y) = (x, y);
        blit_image_to_texture(image, atlas, dest_x as i32, dest_y as i32);

        // Advance to the next available space in the texture
        // Scan the texture atlas left to right, bottom to top
        x += BLOCK_TEXTURE_SIZE;
        if x >= TEXTURE_ATLAS_SIZE {
            x = 0;
            y += BLOCK_TEXTURE_SIZE;
        }

        // Return the x and y coordinates where we put the texture in OpenGL format
        // Between 0.0 and 1.0
        let (dest_x, dest_y) = (dest_x as f32, dest_y as f32);
        (dest_x / TEXTURE_ATLAS_SIZE as f32,
         dest_y / TEXTURE_ATLAS_SIZE as f32,
         (dest_x + BLOCK_TEXTURE_SIZE as f32) / TEXTURE_ATLAS_SIZE as f32,
         (dest_y + BLOCK_TEXTURE_SIZE as f32) / TEXTURE_ATLAS_SIZE as f32)
    };

    let mut face_uvs = HashMap::<BlockID, BlockFaces<UVCoords>>::new();
    for (block, faces) in face_images {
        match faces {
            BlockFaces::All(all) => {
                face_uvs.insert(block, BlockFaces::All(put_image_into_atlas(&mut read_image(all))));
            }
            BlockFaces::Sides { sides, top, bottom } => {
                face_uvs.insert(block, BlockFaces::Sides {
                    sides: put_image_into_atlas(&mut read_image(sides)),
                    top: put_image_into_atlas(&mut read_image(top)),
                    bottom: put_image_into_atlas(&mut read_image(bottom)),
                });
            }
            BlockFaces::Each { top, bottom, front, back, left, right } => {
                face_uvs.insert(block, BlockFaces::Each {
                    top: put_image_into_atlas(&mut read_image(top)),
                    bottom: put_image_into_atlas(&mut read_image(bottom)),
                    front: put_image_into_atlas(&mut read_image(front)),
                    back: put_image_into_atlas(&mut read_image(back)),
                    left: put_image_into_atlas(&mut read_image(left)),
                    right: put_image_into_atlas(&mut read_image(right)),
                });
            }
        }
    }
    face_uvs
}

fn read_image(image_path: &str) -> DynamicImage {
    let img = match image::open(image_path) {
        Ok(img) => img.flipv(), // We flip the y axis for OpenGL
        Err(err) => panic!("Filename: {}, error: {}", image_path, err.to_string())
    };
    match img.color() {
        image::RGBA(8) => {}
        _ => panic!("Texture format not supported")
    };
    img
}

fn blit_image_to_texture(src: &DynamicImage, dest: u32, x: i32, y: i32) {
    gl_call!(gl::TextureSubImage2D(
            dest, 0,
            x, y, src.width() as i32, src.height() as i32,
            gl::RGBA, gl::UNSIGNED_BYTE,
            src.raw_pixels().as_ptr() as *mut c_void));
}