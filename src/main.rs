use opencv::{
    core::{Vec3b, CV_32F, CV_32FC1, CV_32FC3, CV_8UC1}, highgui::{self, WINDOW_AUTOSIZE}, prelude::*, types::{VectorOfVec3f, VectorOfi32, VectorOfu8}, videoio, Error, Result
}; // Note, the namespace of OpenCV is changed (to better or worse). It is no longer one enormous.
use tokio::{sync::mpsc, time::timeout}; // Communicate data
use tokio::task; // threads
use std::{borrow::{Borrow, BorrowMut}, f32::consts::{self, PI}, sync::{Arc, Mutex}, time::Duration};
use image::{imageops::colorops, DynamicImage, GenericImageView, ImageBuffer, Luma, Rgba}; 


const FRAME_TIMEOUT_DURATION = Duration::from_secs(5); // Adjust timeout duration as needed

async fn process_frames(receiver: &mut mpsc::Receiver<Arc<tokio::sync::Mutex<Mat>>>, normalized_kernel: &Vec<Vec<f32>>) {
    println!("IN PROCESS FRAMES");
    loop {
    println!("IN PROCESS FRAMES {:#?}", receiver.recv().await);
        // Attempt to receive a frame from the channel
        let frame = match receiver.recv().await {
            Some(frame) => frame,
            None => {
                println!("Channel closed or no more frames to receive");
                break; // Exit the loop if the channel is closed or no more frames to receive
            }
        };

        // Print a message to indicate that a frame has been received
        println!("Received frame from channel");

        // Lock the frame and perform processing
        let mut locked_frame = frame.lock().await;
        // Perform further processing...
        println!("IN CHANNEL {:#?}", *locked_frame);


        let convolve_value_made = create_convolve_value(&locked_frame, &normalized_kernel);
        let columns = locked_frame.cols();
        let rows = locked_frame.rows();

        create_frame(&mut locked_frame, columns, rows, &convolve_value_made).await;
        println!("Rendered Frames: {:#?}", *locked_frame);
        let _ = highgui::imshow("window", &*locked_frame);
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut cam = videoio::VideoCapture::new(1, videoio::CAP_ANY)?;
    highgui::named_window("window", highgui::WINDOW_FULLSCREEN)?;

    let (sender, mut receiver) = mpsc::channel::<Arc<tokio::sync::Mutex<Mat>>>(1000000);
    let kernel_size = 5;
    let sigma = 1000.0;
    let kernel = generate_gaussian_weight_kernel(kernel_size, sigma);
    let normalized_kernel = Arc::new(normalize_kernel(&kernel));

    // Spawn the processing task to asynchronously receive frames and process them
    let processing_task = task::spawn(async move {
        println!("Moving reciever");
        match receiver.recv().await {
            Some(frame) => {
                println!("Received frame from channel");
                // Process the received frame...
            }
            None => {
                println!("Channel closed or no more frames to receive");
            }
        }
        process_frames(&mut receiver, &normalized_kernel).await;
    });

    // Spawn the camera task to asynchronously read frames and send them through the channel
    task::spawn(async move {
        loop {
            let mut frame = Mat::default();
            cam.read(&mut frame).expect("Camera frame not put to frame");
            // Convert the frame to grayscale
            let grayscale_image = colorops::grayscale(&mat_to_image(&mut frame).expect("Image not grayscale"));

            // Convert the grayscale image to a Mat
            let grayscale_mat = image_buffer_to_opencv_data(&grayscale_image);
            println!("{:#?}", grayscale_mat);

            let frame = Arc::new(tokio::sync::Mutex::new(grayscale_mat));
           
            if let Err(err) = sender.send(Arc::clone(&frame)).await {
                    println!("Error sending frame through channel: {}", err);
                    // Log the error and continue sending frames
            }  // Use `timeout` to limit the time to read a frame
 timeout(FRAME_TIMEOUT_DURATION, async {
                let result = cam.read(&mut frame);
                result
            })
            .await
            .map_err(|_| "Timeout occurred") // Map timeout error to a custom error message
            .and_then(|result| result) // Unwrap the nested Result
            {
                Ok(true) => {
                    // Frame successfully read from the camera
                    if let Err(_) = sender.send(frame.clone()).await {
                        println!("Error sending frame through channel");
                        break; // Channel closed, terminate the task
                    }
                }
                Ok(false) => {
                    // Error occurred while reading the frame
                    println!("Error reading frame from camera");
                }
                Err(err) => {
                    // Timeout occurred
                    println!("Timeout: No frame received from camera ({})", err);
                }
            }
            
        }
    });

    // Wait for the processing task to finish
    let _ = processing_task.await;

    Ok(())
}
fn image_buffer_to_opencv_data(image: &ImageBuffer<image::Luma<u8>, Vec<u8>>) -> Mat {
    // Create a new Mat with the same dimensions as the ImageBuffer
    let (width, height) = image.dimensions();
    let mut data = Mat::new_rows_cols_with_default(height as i32, width as i32, opencv::core::CV_8UC1, opencv::core::Scalar::all(0.0)).unwrap();

    // Iterate over each pixel in the ImageBuffer and assign its value to the corresponding position in the Mat
    for y in 0..height {
        for x in 0..width {
            // Get the pixel value at (x, y) from the ImageBuffer
            let pixel_value = image.get_pixel(x, y)[0];
            // Assign the pixel value to the corresponding position in the Mat
            *data.at_2d_mut::<u8>(y as i32, x as i32).unwrap() = pixel_value;
        }
    }
    data
}
fn mat_to_image(mat: &mut Mat) -> Result<DynamicImage, Error> {
    if mat.channels() != 3 {
        return Err(Error::new(400, "Image must have three channels to be converted (BGR)"));
    }

    let width = mat.cols() as u32;
    let height = mat.rows() as u32;

    let mut rgb_image = ImageBuffer::<Rgba<u8>, Vec<u8>>::new(width, height);

    for y in 0..height {
        for x in 0..width {
            let bgr_pixel = mat.at_2d::<Vec3b>(y as i32, x as i32)?;

            let r = bgr_pixel[2];
            let g = bgr_pixel[1];
            let b = bgr_pixel[0];

            let rgba_pixel = image::Rgba([r, g, b, 255]);
            rgb_image.put_pixel(x, y, rgba_pixel);

        }
    }

    let dynamic_image = DynamicImage::ImageRgba8(rgb_image);
    Ok(dynamic_image)
}

async fn create_frame(output_image: &mut Mat, columns: i32, rows: i32, blurred_pixel_value: &Vec<f32>) {
    // Convert the output image to floating-point representation (CV_32FC1)
    let mut gray_frame = Mat::default();
    let mut index = 0;
    output_image.convert_to(&mut gray_frame, CV_32FC1, 1.0, 0.0).expect("Failed to convert to CV_32FC1");
    
    // Precompute kernel size and center

    for y in 0..rows {
        for x in 0..columns {
            // Assign the blurred pixel value to the output image
            *gray_frame.at_2d_mut::<f32>(y, x).expect("Failed to access pixel") = *blurred_pixel_value.get(index).expect("Pixel blurred cannot be assigned to new grayscale with 32 with 1 channel");
            index+=1;
        }
    }

    // Convert the output image back to CV_8UC1 (grayscale) for display
    gray_frame.convert_to(output_image, CV_8UC1, 1.0, 0.0).expect("Failed to convert to CV_8UC1");
}

// Do the convolve once for a frame 
fn create_convolve_value(frame: &Mat,  kernel: &Vec<Vec<f32>>) -> Vec<f32> {
    let mut convolved_values: Vec<f32> = Vec::with_capacity(1500);
    let mut f32_frame = Mat::default();

    frame.convert_to(&mut f32_frame, CV_32F, 1.0, 0.0).expect("Wrong channels");
    for y in 0..f32_frame.rows() {
        for x in 0..frame.cols() {

            let _ = convolved_values.push(convolve_pixel(&f32_frame, x as usize, y as usize, kernel));
/*             println!("CON: {}", convolve_pixel(&f32_frame, x as usize, y as usize, kernel)); */
        }
    }

    convolved_values
}

fn convolve_pixel(frame: &Mat, x: usize, y: usize, kernel: &Vec<Vec<f32>>) -> f32 {
    let mut blurred_pixel_value = 0.0;
    let kernel_size = kernel.len();
    // println!("KERNEL: {:#?}", kernel);
    // println!("Kernel size: {}", kernel_size);
    let center = kernel_size / 2; // Size of whole image and get the center value 
    // println!("Center kernel: {}", center);
    // let mut check = 0;


    // Convert the data type to 32-bit floating point (CV_32FC1) outside the loop

    for ky in 0..kernel_size {
        for kx in 0..kernel_size {
            let pixel_x = (x as i32 - center as i32 + kx as i32) as i32;
            let pixel_y = (y as i32 - center as i32 + ky as i32) as i32;
            // println!("PIXEL Y: {} | PIXEL X: {}", pixel_y, pixel_x);

            // Check if pixel coordinates are within bounds
            if pixel_x >= 0 && pixel_x < frame.cols() && pixel_y >= 0 && pixel_y < frame.rows() {
                let kernel_value = *kernel.get(ky).and_then(|row| row.get(kx)).expect("Invalid kernel access");
                /* println!("Kernel value: {:#?}", *kernel.get(ky).expect("Y not found for kernel"));
                println!("FRAME HERE IN BLUR COVOLUTE: {:#?}", frame); */
                blurred_pixel_value += *frame.at_2d::<f32>(pixel_y, pixel_x).expect("Failed to access pixel") * kernel_value;
                // println!("BLUR PIXEL VALUE: {}", *frame.at_2d::<f32>(pixel_y, pixel_x).expect("Failed to access pixel"));

            }
        }
                        /* check+=1;
                if check == 5 {
                    panic!("AHH");
                } */

    }

    blurred_pixel_value
}
// Function to generate Gaussian kernel
fn generate_gaussian_weight_kernel(size: usize, sigma: f32) -> Vec<Vec<f32>> {
    let mut kernel = vec![vec![0.0; size]; size];
    let center = size / 2;
    
    for y in 0..size {
        for x in 0..size {
            let distance_x = (x as isize - center as isize) as f32;
            let distance_y = (y as isize - center as isize) as f32;
            let exponent = -(distance_x.powi(2) + distance_y.powi(2)) / (2.0 * sigma.powi(2)) as f32;
            kernel[y][x] = (1.0 / (2.0 * consts::PI * sigma.powi(2))).exp() * exponent;
        }
    }

    // println!("Unnormalized KERNEL: {:#?}", kernel);

    kernel
}

fn normalize_kernel(kernel: &Vec<Vec<f32>>) -> Vec<Vec<f32>> {
    // Calculate the sum of all elements in the 2D vector
    let sum: f32 = kernel.iter().map(|row| row.iter().sum::<f32>()).sum();

    // Normalize the kernel by dividing each element by the sum
    let normalized_kernel: Vec<Vec<f32>> = kernel.iter()
        .map(|row| {
            row.iter()
                .map(|&x| x / sum)
                .collect()
        })
        .collect();

    normalized_kernel
}
