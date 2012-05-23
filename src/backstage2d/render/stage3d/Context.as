package backstage2d.render.stage3d 
{
	import com.unfocus.signalslite.SignalLite;
	
	import flash.display.Stage3D;
	import flash.display3D.Context3D;
	import flash.display3D.Context3DRenderMode;
	
	import flash.events.ErrorEvent;
	import flash.events.Event;
	
	import flash.geom.Matrix3D;
	
	/**
	 * Manages the creation, storage and updating of Context3D.
	 * This is basically a slim facade over Context3D.
	 * @author Kevin Newman
	 */
	public class Context 
	{
		public const created:SignalLite = new SignalLite();
		public const failed:SignalLite = new SignalLite();
		public const lost:SignalLite = new SignalLite();
		
		protected var _width:int// = 0;
		public function get width():int {
			return _width;
		}
		
		protected var _height:int// = 0;
		public function get height():int {
			return _height;
		}
		
		protected var _quality:int// = 0;
		public function get quality():int {
			return _quality;
		}
		
		protected var _checkErrors:Boolean// = false;
		public function set checkErrors( checkErrors:Boolean ):void {
			_checkErrors = checkErrors;
			context3D.enableErrorChecking = checkErrors;
		}
		public function get checkErrors():Boolean {
			return _checkErrors;
		}
		
		public var stage3D:Stage3D;
		public var context3D:Context3D;
		
		public function Context( stage3D:Stage3D ) 
		{
			this.stage3D = stage3D;
			// :NOTE: This fires whenever the context3D is created or lost, for example,
			// after the lock screen is activated on Windows devices.
			// http://help.adobe.com/en_US/FlashPlatform/reference/actionscript/3/flash/display3D/Context3D.html
			stage3D.addEventListener( Event.CONTEXT3D_CREATE, onContextCreate );
			stage3D.addEventListener( ErrorEvent.ERROR, onContextFail );
			
			stage3D.requestContext3D( Context3DRenderMode.AUTO );
		}
		
		protected function onContextFail( event:ErrorEvent ):void
		{
			context3D = null;
			failed.dispatch( event );
		}
		
		protected function onContextCreate( event:Event ):void
		{
			context3D = stage3D.context3D;
			context3D.enableErrorChecking = _checkErrors;
			
			if ( viewMatrix ) {
				context3D.configureBackBuffer( _width, _height, _quality );
			}
			
			created.dispatch( this );
		}
		
		internal var viewMatrix:Matrix3D;
		public function configure( width:int, height:int, quality:int = -1 ):void
		{
			_width = width;
			_height = height;
			_quality = quality > -1 ? quality : _quality;
			
			if ( null !== context3D && context3D.driverInfo !== "Disposed" ) {
				context3D.configureBackBuffer( width, height, _quality, false );
			}
			
			viewMatrix = new Matrix3D();
			viewMatrix.appendTranslation(-width/2, -height/2, 0);
			viewMatrix.appendScale(2.0/width, -2.0/height, 1);
		}
		
		public function verify():Boolean
		{
			if ( null == context3D || context3D.driverInfo == "Disposed" ) {
				lost.dispatch();
				return false;
			}
			return true;
		}
		
	}
}
