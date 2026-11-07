use std::{ffi::OsString, fs, io::Write, path::Path};

pub const SIDE: u32 = 320;
const PIXELS: usize = SIDE as usize * SIDE as usize;

pub fn run(args: &[OsString]) -> Result<(), String> {
    let [model, input, output] = args else {
        return Err("Invalid local segmentation request.".into());
    };
    let input = crate::library::authorized_path(Path::new(input))?;
    let output = Path::new(output);
    let parent = output.parent().ok_or("Missing mask directory.")?;
    let parent = crate::library::authorized_path(parent)?;
    if input.parent() != Some(&parent)
        || input
            .file_name()
            .is_none_or(|name| name != "segment-rgb.raw")
        || output
            .file_name()
            .is_none_or(|name| name != "segment-mask.raw")
        || crate::portable_zip::regular_file(&input)? != (PIXELS * 3) as u64
    {
        return Err("Invalid input for the local segmentation worker.".into());
    }
    let model = super::component::verify(Path::new(model))?;
    let input = fs::read(input).map_err(|e| e.to_string())?;
    let mask = predict(&model, &input)?;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(parent.join("segment-mask.raw"))
        .map_err(|e| e.to_string())?;
    file.write_all(&mask).map_err(|e| e.to_string())
}

#[cfg(windows)]
pub(super) fn predict(model: &Path, rgb: &[u8]) -> Result<Vec<u8>, String> {
    use windows::{
        core::{Interface, HSTRING},
        Win32::System::WinRT::{RoInitialize, RoUninitialize, RO_INIT_MULTITHREADED},
        AI::MachineLearning::{
            LearningModel, LearningModelBinding, LearningModelDevice, LearningModelDeviceKind,
            LearningModelSession, TensorFloat,
        },
    };
    struct Apartment;
    impl Drop for Apartment {
        fn drop(&mut self) {
            unsafe {
                RoUninitialize();
            }
        }
    }
    unsafe { RoInitialize(RO_INIT_MULTITHREADED) }.map_err(|e| e.to_string())?;
    let _apartment = Apartment;
    if rgb.len() != PIXELS * 3 {
        return Err("The input tensor has an invalid size.".into());
    }
    let maximum = f32::from(rgb.iter().copied().max().unwrap_or(0).max(1));
    let means = [0.485, 0.456, 0.406];
    let deviations = [0.229, 0.224, 0.225];
    let mut input = vec![0f32; PIXELS * 3];
    for (index, pixel) in rgb.chunks_exact(3).enumerate() {
        for channel in 0..3 {
            input[channel * PIXELS + index] =
                (f32::from(pixel[channel]) / maximum - means[channel]) / deviations[channel];
        }
    }
    let result = (|| -> windows::core::Result<Vec<f32>> {
        let path = HSTRING::from(model.as_os_str());
        let model = LearningModel::LoadFromFilePath(&path)?;
        let input_name = model.InputFeatures()?.GetAt(0)?.Name()?;
        let output_name = model.OutputFeatures()?.GetAt(0)?.Name()?;
        let device = LearningModelDevice::Create(LearningModelDeviceKind::Cpu)?;
        let session = LearningModelSession::CreateFromModelOnDevice(&model, &device)?;
        let binding = LearningModelBinding::CreateFromSession(&session)?;
        let tensor = TensorFloat::CreateFromShapeArrayAndDataArray(
            &[1, 3, SIDE.into(), SIDE.into()],
            &input,
        )?;
        binding.Bind(&input_name, &tensor)?;
        let output: TensorFloat = session
            .Evaluate(&binding, &HSTRING::new())?
            .Outputs()?
            .Lookup(&output_name)?
            .cast()?;
        let values = output.GetAsVectorView()?;
        if values.Size()? as usize != PIXELS {
            return Ok(Vec::new());
        }
        let mut mask = vec![0f32; PIXELS];
        let count = values.GetMany(0, &mut mask)?;
        mask.truncate(count as usize);
        Ok(mask)
    })()
    .map_err(|e| format!("Windows ML could not run the local U²-Net model: {e}"))?;
    if result.len() != PIXELS || result.iter().any(|value| !value.is_finite()) {
        return Err("Windows ML returned an invalid foreground mask.".into());
    }
    let (low, high) = result
        .iter()
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(low, high), &value| {
            (low.min(value), high.max(value))
        });
    if high - low < 0.00001 {
        return Err("No distinct subject was detected in this image.".into());
    }
    Ok(result
        .into_iter()
        .map(|value| (((value - low) / (high - low)) * 255.0).round() as u8)
        .collect())
}

#[cfg(not(windows))]
fn predict(_: &Path, _: &[u8]) -> Result<Vec<u8>, String> {
    Err("Background removal requires Windows ML.".into())
}
