use crate::InputBackend;

/**
 * 对外显式暴露输入 provider 边界。
 * 当前先复用既有 `InputBackend` 合同，后续再继续拆设备句柄与能力查询。
 */
pub trait InputProvider: InputBackend {}

impl<T> InputProvider for T where T: InputBackend {}
