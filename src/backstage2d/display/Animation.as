package backstage2d.display 
{
	/**
	 * A base class for frame based animations.
	 * @author Kevin Newman
	 */
	public class Animation extends Actor implements IAnimate
	{
		public var frameRate:Number;
		public var totalFrames:int;
		public var currentFrame:int;
		
		public function Animation( width:Number = 0, height:Number = 0, frameRate:Number = 0 ) 
		{
			super( width, height);
			this.frameRate = frameRate;
		}
		
		public function play():void {
			
		}
		
		public function pause():void {
			
		}
		
		public function gotoFrame( frame:int ):void {
			
		}
		
	}
}
