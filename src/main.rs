mod at24c32;
mod ds3231;

use std::cell::RefCell;

use at24c32::At24c32;
use ds3231::{DateTime, Ds3231};

use esp_idf_svc::hal::delay::FreeRtos;
use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::sys::EspError;

use esp_idf_svc::hal::i2c::{I2cConfig, I2cDriver};
use esp_idf_svc::hal::units::FromValueType;

/// Dirección de memoria en la EEPROM donde guardaremos [hora, minuto]
const EEPROM_ADDR_HORA: u16 = 0x0000;

fn main() -> Result<(), EspError> {
    // 1. Inicialización básica del sistema ESP-IDF y del monitor serial
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    // 2. Obtener acceso a los periféricos del ESP32-S3
    let peripherals = Peripherals::take()?;

    // 3. Configurar el bus I2C0 en los pines SDA = GPIO 40 y SCL = GPIO 41 a 100 kHz
    let sda = peripherals.pins.gpio40;
    let scl = peripherals.pins.gpio41;
    let config = I2cConfig::new().baudrate(100_u32.kHz().into());

    // Usamos RefCell para que el RTC y la EEPROM puedan compartir el mismo bus I2C
    let i2c = RefCell::new(I2cDriver::new(peripherals.i2c0, sda, scl, &config)?);

    // 4. Crear las instancias del reloj DS3231M (0x68) y de la memoria AT24C32 (0x57)
    let mut rtc = Ds3231::new(&i2c);
    let mut eeprom = At24c32::new(&i2c);

    // 5. Configurar una fecha y hora inicial en el RTC:
    //    Formato: DateTime::new(año, mes, día, día_semana [1=Lun..7=Dom], hora, minuto, segundo)
    //    (Puedes comentar estas dos líneas una vez que el reloj ya esté en hora)
    let hora_inicial = DateTime::new(2026, 11, 10, 2, 13, 30, 0);
    rtc.set_datetime(&hora_inicial)?;

    // Variable para recordar el último minuto guardado y escribir en EEPROM solo una vez por minuto
    let mut ultimo_minuto_guardado: Option<u8> = None;

    // 6. Bucle principal: se repite cada segundo
    loop {
        // Leer la fecha y hora actual desde el RTC DS3231M
        let dt = rtc.get_datetime()?;

        // Mostrar la fecha y hora actual en el monitor serial
        log::info!("RTC -> {}", dt);

        // Si cambió el minuto respecto al último guardado, escribimos [hora, minuto] en la EEPROM
        if ultimo_minuto_guardado != Some(dt.minute) {
            let datos_hora = [dt.hour, dt.minute];

            // Escribir los 2 bytes (hora y minuto) siempre en la misma dirección (0x0000)
            eeprom.write_bytes(EEPROM_ADDR_HORA, &datos_hora)?;
            ultimo_minuto_guardado = Some(dt.minute);

            // Leer de vuelta los 2 bytes desde la EEPROM para verificar que se guardaron
            let mut leido = [0u8; 2];
            eeprom.read_bytes(EEPROM_ADDR_HORA, &mut leido)?;

            log::info!(
                "EEPROM [0x{:04X}] -> Guardado y leído: {:02}:{:02}",
                EEPROM_ADDR_HORA,
                leido[0],
                leido[1]
            );
        }

        // Esperar 1 segundo (1000 ms) antes de la siguiente lectura
        FreeRtos::delay_ms(1000);
    }
}
