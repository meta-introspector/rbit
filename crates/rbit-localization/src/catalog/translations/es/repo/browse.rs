//! Spanish (`es`) entries for `catalog/repo/browse.rs`.

use crate::catalog::collab::rich::RichWriter;
use crate::catalog::fallback::MessageWriter;
use crate::catalog::repo::browse::RepoBrowseSentence;
use crate::catalog::repo::browse::{RepoBrowseLabel, RepoBrowseMessage};
use crate::plural::Plural;

entries! { Es:
    RepoBrowseLabel => MessageWriter |text| {
        RepoBrowseLabel::SecurityAutomation => text.literal("Análisis automáticos"),
        RepoBrowseLabel::SecurityAutomationHelp => text.literal("Active el análisis de secretos en texto y dependencias npm con package-lock v3. Se analiza tras cambios y periódicamente, sin ejecutar código del repositorio. Los hallazgos solo son visibles para quienes pueden escribir."),
        RepoBrowseLabel::SecuritySecrets => text.literal("Buscar tokens compatibles y claves privadas en texto"),
        RepoBrowseLabel::SecurityDependencies => text.literal("Buscar avisos conocidos de dependencias públicas npm"),
        RepoBrowseLabel::SecurityNetworkConsent => text.literal("Permito enviar nombres y versiones de paquetes públicos a OSV y npm. No se envían archivos fuente, secretos ni credenciales de registros privados."),
        RepoBrowseLabel::SecurityRequireSecrets => text.literal("Exigir análisis completos de secretos para el origen y la base exactos"),
        RepoBrowseLabel::SecurityRequireDependencies => text.literal("Exigir análisis recientes de dependencias para el origen y la base exactos"),
        RepoBrowseLabel::SecurityAutomaticUpdates => text.literal("Proponer automáticamente solicitudes de cambio para actualizaciones compatibles"),
        RepoBrowseLabel::SecurityUpdateVisibility => text.literal("Las solicitudes de cambio heredan la visibilidad del repositorio, incluida la pública. Contienen actualizaciones, nunca valores secretos detectados. Requieren revisión y CI; nunca se fusionan automáticamente."),
        RepoBrowseLabel::SecurityScanNow => text.literal("Analizar la rama predeterminada actual"),
        RepoBrowseLabel::SecurityCoverage => text.literal("Los análisis cubren archivos UTF-8 normales con límites y el archivo de bloqueo npm raíz. Las entradas no compatibles o excesivas y los fallos del proveedor generan evidencia incompleta. Solo se comprueban detectores compatibles; los secretos no se revocan automáticamente. La evidencia de dependencias caduca en 24 horas."),
        RepoBrowseLabel::SecurityRepair => text.literal("Proponer actualización de dependencia"),
        RepoBrowseLabel::SecurityRepairHelp => text.literal("La reparación automática solo admite dependencias directas sin otras dependencias, fijadas a una versión exacta, y actualizaciones de parche seguras. Los espacios de trabajo, alias, cambios transitivos, scripts de instalación y otros casos requieren actualización manual."),
        RepoBrowseLabel::SecurityPackage => text.literal("Paquete npm directo"),
        RepoBrowseLabel::SecurityUpdateHistory => text.literal("Propuestas de actualización de dependencias"),
        RepoBrowseLabel::SecurityPreparedUpdate => text.literal("Rama preparada; reintente para abrir la solicitud de cambio"),
        RepoBrowseLabel::Security => text.literal("Seguridad"),
        RepoBrowseLabel::SecurityHelp => text.literal("Hallazgos de análisis de seguridad. Solo los colaboradores con acceso de escritura pueden verlos."),
        RepoBrowseLabel::SecuritySettings => text.literal("Configuración del análisis"),
        RepoBrowseLabel::SecurityScanner => text.literal("Nombre del analizador (nombre exacto en SARIF)"),
        RepoBrowseLabel::SecurityWorkflow => text.literal("Flujo fiable de la rama predeterminada (opcional)"),
        RepoBrowseLabel::SecurityMinimum => text.literal("Gravedad mínima que bloquea"),
        RepoBrowseLabel::SecurityRequire => text.literal("Exigir análisis completos de origen y base antes de fusionar hallazgos nuevos"),
        RepoBrowseLabel::SecuritySettingsHelp => text.literal("Guardar invalida los análisis anteriores. Este requisito no se puede omitir; los administradores pueden desactivarlo aquí."),
        RepoBrowseLabel::SecuritySave => text.literal("Guardar configuración"),
        RepoBrowseLabel::SecurityScan => text.literal("Análisis"),
        RepoBrowseLabel::SecurityView => text.literal("Ver análisis"),
        RepoBrowseLabel::SecurityEmpty => text.literal("Aún no hay análisis. Active el análisis automático o configure un analizador externo y cargue su informe SARIF."),
        RepoBrowseLabel::SecurityHistorical => text.literal("Este informe corresponde a un commit diferente al de la rama predeterminada actual."),
        RepoBrowseLabel::SecurityStale => text.literal("Este análisis ha caducado o es anterior a la configuración actual y no cumple el requisito para fusionar."),
        RepoBrowseLabel::SecurityNoFindings => text.literal("No hay hallazgos en este informe. Un informe pendiente o incompleto no demuestra que el análisis esté limpio."),
        RepoBrowseLabel::SecurityComplete => text.literal("Completo"),
        RepoBrowseLabel::SecurityIncomplete => text.literal("Incompleto"),
        RepoBrowseLabel::SecurityPending => text.literal("Pendiente"),
        RepoBrowseLabel::SecurityCritical => text.literal("Crítica"),
        RepoBrowseLabel::SecurityHigh => text.literal("Alta"),
        RepoBrowseLabel::SecurityMedium => text.literal("Media"),
        RepoBrowseLabel::SecurityLow => text.literal("Baja"),
        RepoBrowseLabel::SecurityInfo => text.literal("Informativa"),
        RepoBrowseLabel::SecurityDismiss => text.literal("Descartar hallazgo"),
        RepoBrowseLabel::SecurityReopen => text.literal("Reabrir hallazgo"),
        RepoBrowseLabel::SecurityReason => text.literal("Motivo para descartar"),
        RepoBrowseLabel::SecurityNote => text.literal("Nota de auditoría"),
        RepoBrowseLabel::SecurityFalsePositive => text.literal("Falso positivo"),
        RepoBrowseLabel::SecurityAcceptedRisk => text.literal("Riesgo aceptado"),
        RepoBrowseLabel::SecurityTestCode => text.literal("Usado en pruebas"),
        RepoBrowseLabel::SecurityPrevious => text.literal("Hallazgos anteriores"),
        RepoBrowseLabel::SecurityNext => text.literal("Más hallazgos"),
        RepoBrowseLabel::CodeSearch => text.literal("Buscar código"),
        RepoBrowseLabel::CodeSearchHelp => text.literal("Busca texto literal que distingue mayúsculas de minúsculas en la rama predeterminada."),
        RepoBrowseLabel::CodeSearchText => text.literal("Texto de búsqueda"),
        RepoBrowseLabel::CodeSearchPath => text.literal("Prefijo de ruta (opcional)"),
        RepoBrowseLabel::CodeSearchLanguage => text.literal("Lenguaje (opcional)"),
        RepoBrowseLabel::CodeSearchLimited => text.literal("La búsqueda alcanzó un límite. Acota el texto o los filtros para encontrar más coincidencias."),
        RepoBrowseLabel::CodeSearchSkipped => text.literal("Archivos binarios, de gran tamaño o sin codificación UTF-8 omitidos:"),
        RepoBrowseLabel::CodeSearchEmpty => text.literal("No se encontraron líneas coincidentes."),
        RepoBrowseLabel::CodeSearchCommit => text.literal("Commit buscado:"),
        RepoBrowseLabel::SigningOpenPgp => text.literal("OpenPGP"),
        RepoBrowseLabel::SignatureTitle => text.literal("Verificación de firma"),
        RepoBrowseLabel::SignatureCheck => text.literal("Verificar firma"),
        RepoBrowseLabel::SignatureUnsigned => text.literal("Sin firma"),
        RepoBrowseLabel::SignatureVerified => text.literal("Verificada con las claves del repositorio"),
        RepoBrowseLabel::SignatureUnknown => text.literal("La clave de firma no es de confianza"),
        RepoBrowseLabel::SignatureInvalid => text.literal("Firma inválida o caducada"),
        RepoBrowseLabel::SignatureUnavailable => text.literal("Verificación no disponible"),
        RepoBrowseLabel::SignatureTrustNote => text.literal("La verificación demuestra que una clave de confianza del repositorio firmó estos bytes exactos. No verifica la cuenta del autor ni la seguridad del código."),
        RepoBrowseLabel::SigningKeys => text.literal("Claves de firma de confianza"),
        RepoBrowseLabel::SigningKeysHelp => text.literal("Los administradores eligen las claves públicas de confianza del repositorio. Al eliminar una clave se revoca su confianza para las verificaciones posteriores. Máximo de 16 claves."),
        RepoBrowseLabel::SigningKeyName => text.literal("Nombre de la clave"),
        RepoBrowseLabel::SigningKeyKind => text.literal("Formato de la clave"),
        RepoBrowseLabel::SigningKeyMaterial => text.literal("Clave pública"),
        RepoBrowseLabel::SigningKeyAdd => text.literal("Confiar en la clave pública"),
        RepoBrowseLabel::SigningKeyRemove => text.literal("Retirar confianza"),

        RepoBrowseLabel::About => text.literal("Acerca de"),
        RepoBrowseLabel::NoDescription => text.literal("Sin descripción."),
        RepoBrowseLabel::Visibility => text.literal("Visibilidad"),
        RepoBrowseLabel::History => text.literal("Historial"),
        RepoBrowseLabel::CommitHistory => text.literal("Historial de commits"),
        RepoBrowseLabel::AddFile => text.literal("Añadir archivo"),
        RepoBrowseLabel::UploadFiles => text.literal("Subir archivos"),
        RepoBrowseLabel::Files => text.literal("Archivos"),
        RepoBrowseLabel::ParentDirectory => text.literal("Carpeta superior"),
        RepoBrowseLabel::NameColumn => text.literal("Nombre"),
        RepoBrowseLabel::LastCommitMessageColumn => text.literal("Último mensaje de commit"),
        RepoBrowseLabel::LastCommitDateColumn => text.literal("Fecha del último commit"),
        RepoBrowseLabel::Path => text.literal("Ruta"),
        RepoBrowseLabel::Latest => text.literal("Más reciente"),
        RepoBrowseLabel::YourAccess => text.literal("Tu acceso"),
        RepoBrowseLabel::LeaveRepository => text.literal("Abandonar el repositorio"),
        RepoBrowseLabel::Languages => text.literal("Lenguajes"),
        RepoBrowseLabel::RepositoryLinks => text.literal("Enlaces del repositorio"),
        RepoBrowseLabel::StoredWithLfs => text.literal("Almacenado con Git LFS"),
        RepoBrowseLabel::LfsObjectMissing => text.literal("El objeto no está en este servidor"),
        RepoBrowseLabel::EditThisFile => text.literal("Editar este archivo"),
        RepoBrowseLabel::DeleteThisFile => text.literal("Eliminar este archivo"),
        RepoBrowseLabel::Blame => text.literal("Autoría"),
        RepoBrowseLabel::Raw => text.literal("Sin formato"),
        RepoBrowseLabel::Download => text.literal("Descargar"),
        RepoBrowseLabel::BinaryNotShown => text.literal("No se muestra el archivo binario."),
        RepoBrowseLabel::TooLargeToDisplay => text.literal("Este archivo es demasiado grande para mostrarlo."),
        RepoBrowseLabel::ViewRaw => text.literal("Ver sin formato"),
        RepoBrowseLabel::RepositoryDocuments => text.literal("Documentos del repositorio"),
        RepoBrowseLabel::DocumentOutline => text.literal("Esquema del documento"),
        RepoBrowseLabel::FilterHeadings => text.literal("Filtrar encabezados"),
        RepoBrowseLabel::DocumentHeadings => text.literal("Encabezados del documento"),
        RepoBrowseLabel::NoMatchingHeadings => text.literal("No hay encabezados coincidentes."),
        RepoBrowseLabel::EmptyTitle => text.literal("Este repositorio está vacío"),
        RepoBrowseLabel::EmptyLead => {
            text.literal("Envía un proyecto existente o empieza uno nuevo desde la línea de comandos.");
        }
        RepoBrowseLabel::RepositoryUrl => text.literal("URL del repositorio"),
        RepoBrowseLabel::CopyRepositoryUrl => text.literal("Copiar la URL del repositorio"),
        RepoBrowseLabel::SshRepositoryUrl => text.literal("URL SSH del repositorio"),
        RepoBrowseLabel::CopySshRepositoryUrl => text.literal("Copiar la URL SSH del repositorio"),
        RepoBrowseLabel::CreateNewRepository => text.literal("Crear un repositorio nuevo"),
        RepoBrowseLabel::PushExistingRepository => text.literal("Enviar un repositorio existente"),
        RepoBrowseLabel::CloseFileBrowser => text.literal("Cerrar el explorador de archivos"),
        RepoBrowseLabel::LanguageFiles => text.literal("Archivos del lenguaje"),
        RepoBrowseLabel::SyncFork => text.literal("Sincronizar bifurcación"),
        RepoBrowseLabel::CompareUpstream => text.literal("Comparar con el repositorio de origen"),
        RepoBrowseLabel::ForkUpToDate => text.literal("Al día con"),
        RepoBrowseLabel::NoFilesForLanguage => text.literal("No se encontraron archivos de este lenguaje."),
        RepoBrowseLabel::NoMatchingFiles => text.literal("No hay archivos coincidentes."),
        RepoBrowseLabel::FileContent => text.literal("Contenido del archivo"),
        RepoBrowseLabel::SelectFileToPreview => text.literal("Selecciona un archivo para ver su contenido."),
        RepoBrowseLabel::SearchThisListing => text.literal("Buscar en esta lista"),
        RepoBrowseLabel::SearchNames => text.literal("Buscar nombres"),
        RepoBrowseLabel::Search => text.literal("Buscar"),
        RepoBrowseLabel::ListingPages => text.literal("Páginas de la lista"),
        RepoBrowseLabel::Previous => text.literal("Anterior"),
        RepoBrowseLabel::Next => text.literal("Siguiente"),
        RepoBrowseLabel::BranchesAndTags => text.literal("Ramas y tags"),
        RepoBrowseLabel::BrowseBranchesAndTags => text.literal("Explorar ramas y tags"),
        RepoBrowseLabel::FindBranchOrTag => text.literal("Buscar una rama o un tag"),
        RepoBrowseLabel::Find => text.literal("Buscar"),
        RepoBrowseLabel::Branch => text.literal("Rama"),
        RepoBrowseLabel::Tag => text.literal("Tag"),
        RepoBrowseLabel::Default => text.literal("predeterminada"),
        RepoBrowseLabel::NoMatchingRefs => text.literal("No hay ramas ni tags coincidentes."),
        RepoBrowseLabel::NoForksYet => text.literal("Aún no hay bifurcaciones"),
        RepoBrowseLabel::ForksEmpty => text.literal(
            "Las bifurcaciones son copias de este repositorio en las que otras personas trabajan por su cuenta.",
        ),
        RepoBrowseLabel::ForkLead => text.literal(
            "Una bifurcación es tu propia copia de las ramas y los tags del repositorio. Conserva la visibilidad \
             del original.",
        ),
        RepoBrowseLabel::ForkLeadPrivate => text.literal(
            "Una bifurcación es tu propia copia de las ramas y los tags del repositorio. Conserva la visibilidad \
             del original: esta seguirá siendo privada.",
        ),
        RepoBrowseLabel::Owner => text.literal("Propietario"),
        RepoBrowseLabel::Name => text.literal("Nombre"),
        RepoBrowseLabel::CreateFork => text.literal("Crear bifurcación"),
        RepoBrowseLabel::Cancel => text.literal("Cancelar"),
        RepoBrowseLabel::NoStargazersYet => text.literal("Aún nadie lo ha marcado con estrella"),
        RepoBrowseLabel::StargazersEmpty => {
            text.literal("Aquí aparecen las personas que marcan este repositorio con estrella.");
        }
        RepoBrowseLabel::RepositoryActivity => text.literal("Actividad del repositorio"),
        RepoBrowseLabel::GoToFile => text.literal("Ir al archivo"),
        RepoBrowseLabel::ClearFileFilters => text.literal("Borrar filtros"),
        RepoBrowseLabel::MatchingFiles => text.literal("Archivos coincidentes"),
        RepoBrowseLabel::LoadingFiles => text.literal("Cargando archivos…"),
        RepoBrowseLabel::UnableToLoadFiles => text.literal("No se pudieron cargar los archivos. Pulsa Intro para buscar."),
        RepoBrowseLabel::SeeAllMatches => text.literal("Ver todas las coincidencias"),
        RepoBrowseLabel::NoFilesInRevision => text.literal("Esta revisión no tiene archivos."),
        RepoBrowseLabel::TooManyFilesToSearch => text.literal("Esta revisión tiene demasiados archivos para buscar en todos."),
    }
    RepoBrowseMessage<'a> => MessageWriter |text| {
        RepoBrowseMessage::BranchNoun { count } => text.literal(match text.plural(count as u64) {
            Plural::One => "rama",
            _ => "ramas",
        }),
        RepoBrowseMessage::TagNoun { count } => text.literal(match text.plural(count as u64) {
            Plural::One => "tag",
            _ => "tags",
        }),
        RepoBrowseMessage::CollaboratorAccess { permission } => {
            text.literal("Acceso de ");
            text.argument(permission);
            text.literal(" como colaborador.");
        }
        RepoBrowseMessage::LineCount { count } => {
            text.number(count as u64);
            text.literal(match text.plural(count as u64) {
                Plural::One => " línea",
                _ => " líneas",
            });
        }
        RepoBrowseMessage::LanguageFiles { language } => {
            text.literal("Archivos de ");
            text.argument(language);
        }
        RepoBrowseMessage::ListingResults { total, page, pages } => {
            text.number(total as u64);
            text.literal(match text.plural(total as u64) {
                Plural::One => " resultado · ",
                _ => " resultados · ",
            });
            text.number(page as u64);
            text.literal(" / ");
            text.number(pages as u64);
        }
        RepoBrowseMessage::RefPage { page, pages, total } => {
            text.number(page as u64);
            text.literal(" / ");
            text.number(pages as u64);
            text.literal(" · ");
            text.number(total as u64);
            text.literal(match text.plural(total as u64) {
                Plural::One => " referencia",
                _ => " referencias",
            });
        }
        RepoBrowseMessage::ForkDivergence { ahead, behind } => {
            let count = |n: usize, suffix, text: &mut MessageWriter| {
                text.number(n as u64);
                if n >= 10_000 {
                    text.literal("+");
                }
                text.literal(match text.plural(n as u64) {
                    Plural::One => " commit ",
                    _ => " commits ",
                });
                text.literal(suffix);
            };
            if ahead > 0 {
                count(ahead, "por delante", text);
            }
            if ahead > 0 && behind > 0 {
                text.literal(" y ");
            }
            if behind > 0 {
                count(behind, "por detrás", text);
            }
            text.literal(" de");
        }
        RepoBrowseMessage::ForkTitle { owner, name } => {
            text.literal("Bifurcar ");
            text.argument(owner);
            text.literal("/");
            text.argument(name);
        }
        RepoBrowseMessage::CommitNoun { count } => text.literal(match text.plural(count as u64) {
            Plural::One => "commit",
            _ => "commits",
        }),
        RepoBrowseMessage::NoFileMatches { query } => {
            text.literal("Ningún archivo coincide con «");
            text.argument(query);
            text.literal("».");
        }
        RepoBrowseMessage::MoreFileMatches { count } => {
            text.literal("Coincidencias adicionales: ");
            text.number(count as u64);
            text.literal(". Escribe más del nombre para acotar la lista.");
        }
    }
    RepoBrowseSentence => RichWriter |text| {
        RepoBrowseSentence::EmptySshNote => {
            text.literal("Los comandos siguientes usan HTTPS. Para enviar por SSH, usa la URL SSH y añade una clave en ");
            text.slot(0);
            text.literal(".");
        }
    }
}
